#!/usr/bin/env bash
set -euo pipefail
if [[ ${1:-} != --inside ]]; then
    test_root=$(mktemp -d /tmp/waniengine-rust-test.XXXXXX)
    manager_binary=${1:-$(realpath "$(dirname "$0")/../target/debug/waniengine")}
    [[ -x "$manager_binary" ]]
    mkdir -p "$test_root"/{config,data,state,cache,runtime}
    chmod 700 "$test_root/runtime"
    export XDG_CONFIG_HOME="$test_root/config" XDG_DATA_HOME="$test_root/data"
    export XDG_STATE_HOME="$test_root/state" XDG_CACHE_HOME="$test_root/cache"
    export XDG_RUNTIME_DIR="$test_root/runtime"
    unset DISPLAY WAYLAND_DISPLAY SESSION_MANAGER DBUS_SESSION_BUS_ADDRESS
    printf 'Isolated session: %s\n' "$test_root"
    exec dbus-run-session -- bash "$0" --inside "$test_root" "$manager_binary"
fi
test_root=$2
manager_binary=$3
[[ "$XDG_RUNTIME_DIR" == "$test_root/runtime" && "$test_root" == /tmp/waniengine-rust-test.* ]]
ulimit -c 0
export QT_FORCE_STDERR_LOGGING=1 KDE_DEBUG=1
unset QT_QUICK_BACKEND
export QSG_RHI_BACKEND=opengl
if [[ ${WANIENGINE_TEST_GPU:-0} == 1 ]]; then
    unset LIBGL_ALWAYS_SOFTWARE __EGL_VENDOR_LIBRARY_FILENAMES QT_FFMPEG_DECODING_HW_DEVICE_TYPES
else
    export LIBGL_ALWAYS_SOFTWARE=1 __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json
    export QT_FFMPEG_DECODING_HW_DEVICE_TYPES=,
fi
export XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland
export PIPEWIRE_RUNTIME_DIR="$XDG_RUNTIME_DIR" PIPEWIRE_REMOTE=pipewire-0
export PULSE_RUNTIME_PATH="$XDG_RUNTIME_DIR/pulse" PULSE_SERVER="unix:$XDG_RUNTIME_DIR/pulse/native"
audio_pid='' pulse_pid='' wireplumber_pid='' kwin_pid='' plasma_pid=''
cleanup() {
    for child in "$plasma_pid" "$kwin_pid" "$wireplumber_pid" "$pulse_pid" "$audio_pid"; do
        [[ -z "$child" ]] || kill "$child" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT
# Private sound server, with synthetic sinks only. Hardware monitors are disabled.
mkdir -p "$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d"
cat >"$XDG_CONFIG_HOME/wireplumber/wireplumber.conf.d/99-isolated.conf" <<'EOF'
wireplumber.profiles = {
  main = {
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez-midi = disabled
    monitor.v4l2 = disabled
    monitor.libcamera = disabled
  }
}
EOF
pipewire >"$test_root/pipewire.log" 2>&1 &
audio_pid=$!
for ((i=0;i<100;i++)); do
    [[ -S "$XDG_RUNTIME_DIR/pipewire-0" ]] && break
    sleep 0.05
 done
pipewire-pulse >"$test_root/pipewire-pulse.log" 2>&1 &
pulse_pid=$!
wireplumber >"$test_root/wireplumber.log" 2>&1 &
wireplumber_pid=$!
for ((i=0;i<100;i++)); do
    [[ -S "$XDG_RUNTIME_DIR/pulse/native" ]] && break
    sleep 0.05
done
pactl load-module module-null-sink sink_name=waniengine-test-a sink_properties=device.description=WaniEngineTestA >"$test_root/sink-a"
pactl load-module module-null-sink sink_name=waniengine-test-b sink_properties=device.description=WaniEngineTestB >"$test_root/sink-b"
pactl set-default-sink waniengine-test-a
mkdir -p "$XDG_DATA_HOME/applications"
cat >"$XDG_DATA_HOME/applications/org.waniengine.WaniEngine.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=WaniEngine isolated tests
Exec="$manager_binary"
NoDisplay=true
X-KDE-Wayland-Interfaces=org_kde_plasma_window_management
X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
EOF
kbuildsycoca6 --noincremental >"$test_root/service-cache.log" 2>&1
# Neither compositor nor shell connects to the user's Wayland socket or session bus.
QT_QPA_PLATFORM=offscreen kwin_wayland --virtual --socket waniengine-test --width 1920 --height 1080 --output-count 2 --no-global-shortcuts --no-lockscreen --no-kactivities >"$test_root/kwin.log" 2>&1 &
kwin_pid=$!
plasma_pid=''
for ((i=0;i<150;i++)); do
    [[ -S "$XDG_RUNTIME_DIR/waniengine-test" ]] && break
    kill -0 "$kwin_pid" || { cat "$test_root/kwin.log"; exit 1; }
    sleep 0.1
done
[[ -S "$XDG_RUNTIME_DIR/waniengine-test" ]]
export WAYLAND_DISPLAY=waniengine-test QT_QPA_PLATFORM=wayland
dbus-update-activation-environment WAYLAND_DISPLAY QT_QPA_PLATFORM XDG_CURRENT_DESKTOP XDG_SESSION_TYPE QT_QUICK_BACKEND QT_FORCE_STDERR_LOGGING KDE_DEBUG
plasmashell --no-respawn >"$test_root/plasma.log" 2>&1 &
plasma_pid=$!
for ((i=0;i<150;i++)); do
    if qdbus6 org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.evaluateScript 'print(desktops().length)' >"$test_root/ready" 2>/dev/null; then
        [[ $(cat "$test_root/ready") != 0 ]] && break
    fi
    kill -0 "$plasma_pid" || { cat "$test_root/plasma.log"; exit 1; }
    sleep 0.1
done
[[ $(cat "$test_root/ready") != 0 ]]
ffmpeg -loglevel error -f lavfi -i 'testsrc2=size=160x120:rate=10' -t 1 -y "$test_root/animation.gif"
ffmpeg -loglevel error -f lavfi -i 'color=c=purple:s=2048x2048' -frames:v 1 -y "$test_root/large.png"
ffmpeg -loglevel error -f lavfi -i 'testsrc2=size=3840x2160:rate=10' -f lavfi -i 'sine=frequency=440:sample_rate=48000' -t 1 -c:a aac -c:v libx264 -pix_fmt yuv420p -y "$test_root/video.mp4"
ffmpeg -loglevel error -f lavfi -i 'sine=frequency=440:sample_rate=48000' -t 0.25 -y "$test_root/sound.wav"
ffmpeg -loglevel error -f lavfi -i 'sine=frequency=660:sample_rate=48000' -t 3 -y "$test_root/sound-long.wav"
export WANIENGINE_ISOLATED_TEST_ROOT="$test_root"
export WANIENGINE_DATA_ROOT="$test_root/library"
export WANIENGINE_INSTALLED_SCENE="$test_root/missing-scene.json"
export WANIENGINE_TEST_PLASMA_PID="$plasma_pid"
"$manager_binary" --redesign --verify-backend "$test_root" >"$test_root/results.json" 2>"$test_root/tests.log" || { cat "$test_root/tests.log"; tail -60 "$test_root/plasma.log"; exit 1; }
kill -0 "$plasma_pid"
if rg -n 'QDBusMarshaller|cannot add a null|Error loading QML|Cannot assign|ReferenceError|TypeError|ASSERT|Aborted' "$test_root/plasma.log" "$test_root/tests.log"; then exit 1; fi
cat "$test_root/tests.log"
cat "$test_root/results.json"
printf 'Isolated desktop verification passed. Logs: %s\n' "$test_root"
