#!/usr/bin/env bash
set -euo pipefail
ulimit -c 0
if [[ ${1:-} != --inside ]]; then
    test_root=$(mktemp -d /tmp/wani-backend-check.XXXXXX)
    binary=$(realpath "${1:-$(dirname "$0")/../target/debug/waniengine-egui}")
    mkdir -p "$test_root"/{home/Desktop,runtime,config,data,state,cache}
    chmod 700 "$test_root/runtime"
    export HOME="$test_root/home" XDG_RUNTIME_DIR="$test_root/runtime" XDG_CONFIG_HOME="$test_root/config" XDG_DATA_HOME="$test_root/data" XDG_STATE_HOME="$test_root/state" XDG_CACHE_HOME="$test_root/cache"
    printf 'WaniEngine private desktop icon\n' >"$test_root/home/Desktop/WANI_TEST.txt"
    printf 'XDG_DESKTOP_DIR="$HOME/Desktop"\n' >"$test_root/config/user-dirs.dirs"
    # Suppress the first-login tour in this disposable HOME. It otherwise hides
    # the centre of both reference captures, invalidating the wallpaper check.
    printf '[General]\nLastSeenVersion=999.0.0\nShowUpdatePage=false\n' >"$test_root/config/plasma-welcomerc"
    export WANIENGINE_ISOLATED_TEST_ROOT="$test_root" SDL_AUDIODRIVER=dummy
    export WANIENGINE_VERIFY_COMPOSITION=1
    unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET DBUS_SESSION_BUS_ADDRESS SESSION_MANAGER PULSE_SERVER PIPEWIRE_REMOTE
    printf 'Private desktop verification: %s\n' "$test_root"
    exec dbus-run-session -- bash "$0" --inside "$test_root" "$binary" 2>"$test_root/session.log"
fi
root=$2; binary=$3
[[ "$root" == /tmp/wani-backend-check.* && "$XDG_RUNTIME_DIR" == "$root/runtime" ]]
export QT_LOGGING_RULES="kwin_scripting.debug=true;js.debug=true" QT_FORCE_STDERR_LOGGING=1
export QT_QPA_PLATFORM=offscreen LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=1 XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland
kwin_wayland --virtual --socket wani-backend --width 1280 --height 720 --output-count 2 --no-lockscreen --no-global-shortcuts --no-kactivities >"$root/kwin.log" 2>&1 &
kwin_pid=$!; shell_pid=''
cleanup(){ [[ -z "$shell_pid" ]] || kill "$shell_pid" 2>/dev/null || true;kill "$kwin_pid" 2>/dev/null || true;wait 2>/dev/null || true; }
trap cleanup EXIT
for ((i=0;i<150;i++));do [[ -S "$XDG_RUNTIME_DIR/wani-backend" ]] && break; kill -0 "$kwin_pid";sleep .1;done
export WAYLAND_DISPLAY=wani-backend QT_QPA_PLATFORM=wayland
# Private activation environment; no host session bus is inherited.
dbus-update-activation-environment WAYLAND_DISPLAY QT_QPA_PLATFORM XDG_CURRENT_DESKTOP XDG_SESSION_TYPE HOME XDG_RUNTIME_DIR XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME
plasmashell --no-respawn >"$root/plasma.log" 2>&1 & shell_pid=$!
export WANIENGINE_TEST_SHELL_PID="$shell_pid"
for ((i=0;i<150;i++));do
 if qdbus6 org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.evaluateScript 'print(desktops().length)' >"$root/ready" 2>/dev/null && [[ $(cat "$root/ready") != 0 ]];then break;fi
 kill -0 "$shell_pid";sleep .1
done
# Wait for Folder View's asynchronous directory model, not merely the D-Bus name.
sleep 3
ffmpeg -nostdin -v error -f lavfi -i 'testsrc2=size=640x360:rate=15' -f lavfi -i 'sine=frequency=440:sample_rate=48000' -t 3 -threads 1 -c:v libx264 -preset ultrafast -c:a aac -y "$root/video.mp4"
ffmpeg -nostdin -v error -f lavfi -i 'color=c=purple:size=640x360' -frames:v 1 -threads 1 -y "$root/still.png"
printf invalid >"$root/invalid.mp4"
sha256sum "$binary" >"$root/binary.sha256"
if [[ ${WANIENGINE_TEST_CRASH:-0} == 1 ]];then
    # The fixture is only the victim: no scenario may add harness-owned children or move outputs.
    env -u WANIENGINE_TEST_SWAPS -u WANIENGINE_TEST_HOTPLUG -u WANIENGINE_TEST_FINITE_VIDEO -u WANIENGINE_TEST_SHELL_RESTART -u WANIENGINE_TEST_EXTERNAL_CHANGE WANIENGINE_CRASH_FIXTURE=1 "$binary" --verify-backend "$root" >"$root/crash-manager.log" 2>&1 &
    manager=$!
    python3 "$(dirname "$0")/../tests/desktops/crash-manager.py" "$root" "$manager"
    wait "$manager" 2>/dev/null || true
fi
timeout "$(( ${WANIENGINE_SOAK_SECONDS:-0} + 240 ))" "$binary" --verify-backend "$root" >"$root/manager.log" 2>&1 || { cat "$root/manager.log";tail -60 "$root/plasma.log";exit 1;}
cat "$root/report.json"
