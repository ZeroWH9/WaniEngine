#!/usr/bin/env bash
set -euo pipefail
ulimit -c 0
mode=${1:?gnome or xfce}
binary=${WANIENGINE_TEST_BINARY:-/workspace/target/debug/waniengine-egui}
if [[ ${2:-} != inside ]];then
 root=$(mktemp -d /tmp/wani-backend-check.XXXXXX)
 mkdir -p "$root"/{home/Desktop,runtime,config,data,state,cache};chmod 700 "$root/runtime"
 export HOME="$root/home" XDG_RUNTIME_DIR="$root/runtime" XDG_CONFIG_HOME="$root/config" XDG_DATA_HOME="$root/data" XDG_STATE_HOME="$root/state" XDG_CACHE_HOME="$root/cache"
 export WANIENGINE_ISOLATED_TEST_ROOT="$root" LIBGL_ALWAYS_SOFTWARE=1 LP_NUM_THREADS=1 SDL_AUDIODRIVER=dummy NO_AT_BRIDGE=1
 unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET DBUS_SESSION_BUS_ADDRESS SESSION_MANAGER
 exec dbus-run-session -- bash "$0" "$mode" inside "$root"
fi
root=$3
pids=()
cleanup(){
 for pid in "${pids[@]}";do kill "$pid" 2>/dev/null || true;done
 mkdir -p "/results/$mode"
 cp "$root"/*.log "$root"/*.png "$root"/report.json "/results/$mode/" 2>/dev/null || true
}
trap cleanup EXIT
if [[ "$mode" == gnome ]];then
 export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$root/system-bus"
 dbus-daemon --session --address "$DBUS_SYSTEM_BUS_ADDRESS" --fork --print-pid >"$root/system-bus.pid"
 pids+=("$(cat "$root/system-bus.pid")")
 export XDG_CURRENT_DESKTOP=GNOME XDG_SESSION_TYPE=wayland WAYLAND_DISPLAY=wani-gnome
 test_extension="$XDG_DATA_HOME/gnome-shell/extensions/wani-private-capture@test.invalid"
 mkdir -p "$test_extension";cp /workspace/tests/desktops/capture-extension/* "$test_extension/"
 gsettings set org.gnome.shell enabled-extensions "['wani-private-capture@test.invalid']"
 gsettings set org.gnome.shell disable-user-extensions false
 gsettings set org.gnome.shell enabled-extensions "['wani-private-capture@test.invalid']"
 if [[ ${WANIENGINE_TEST_DING:-0} == 1 ]];then
  vendor=/workspace/target/desktop-matrix/vendor/desktop-icons-ng-51.1.1
  extension="$XDG_DATA_HOME/gnome-shell/extensions/ding@rastersoft.com"
  mkdir -p "$extension";cp -a "$vendor"/. "$extension"/
  glib-compile-schemas "$extension/schemas"
  printf 'Desktop icon fixture\n' >"$HOME/Desktop/WANI_TEST.txt"
  printf 'XDG_DESKTOP_DIR="$HOME/Desktop"\n' >"$XDG_CONFIG_HOME/user-dirs.dirs"
  gsettings set org.gnome.shell enabled-extensions "['wani-private-capture@test.invalid','ding@rastersoft.com']"
 fi
 gnome-shell --headless --wayland --virtual-monitor 1280x720 --wayland-display wani-gnome --debug-control >"$root/shell.log" 2>&1 & pids+=("$!")
 for ((i=0;i<200;i++));do [[ -S "$XDG_RUNTIME_DIR/wani-gnome" ]] && break;kill -0 "${pids[1]}";sleep .1;done
 for ((i=0;i<100;i++));do
  socket=$(find /tmp/.X11-unix -name 'X*' -type s -print 2>/dev/null | sort -V | head -1 || true)
  [[ -n "$socket" ]] && break;sleep .1
 done
 [[ -n "$socket" ]] || { cat "$root/shell.log";exit 1; }
 export DISPLAY=":${socket##*/X}"
 auth=$(find "$XDG_RUNTIME_DIR" -name '*Xwaylandauth*' -type f -print -quit)
 [[ -z "$auth" ]] || export XAUTHORITY="$auth"
 for ((i=0;i<10;i++));do timeout 2 xdpyinfo >/dev/null 2>&1 && break;sleep .1;done
 gnome-shell --version >"$root/version.log"
 # The test-only extension allows capture on this private bus. No security
 # property of the user's Shell or session is read or modified.
 sleep 2
 gdbus call --session --dest org.gnome.Shell --object-path /org/gnome/Shell --method org.gnome.Shell.Eval 'Main.overview.hide();' >"$root/overview.log"
 if [[ ${WANIENGINE_TEST_DING:-0} == 1 ]];then
  export WANIENGINE_VERIFY_COMPOSITION=1
  cat >"$root/capture" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
gdbus call --session --dest org.gnome.Shell.Screenshot --object-path /org/gnome/Shell/Screenshot --method org.gnome.Shell.Screenshot.Screenshot false false "$1"
SH
  chmod +x "$root/capture";export WANIENGINE_TEST_SCREENSHOT="$root/capture"
 fi
 sleep 3
elif [[ "$mode" == xfce ]];then
 export XDG_CURRENT_DESKTOP=XFCE XDG_SESSION_TYPE=x11 DISPLAY=:91 WANIENGINE_EXPECT_UNSUPPORTED=1
 Xvfb :91 -screen 0 1280x720x24 -nolisten tcp -ac >"$root/xvfb.log" 2>&1 &pids+=("$!")
 for ((i=0;i<10;i++));do timeout 2 xdpyinfo >/dev/null 2>&1 && break;sleep .1;done
 xfwm4 --compositor=off >"$root/wm.log" 2>&1 &pids+=("$!")
 xfdesktop >"$root/desktop.log" 2>&1 &pids+=("$!")
else exit 2;fi
ffmpeg -nostdin -v error -f lavfi -i 'testsrc2=size=640x360:rate=15' -f lavfi -i 'sine=frequency=440:sample_rate=48000' -t 3 -threads 1 -c:v libx264 -preset ultrafast -c:a aac -y "$root/video.mp4"
ffmpeg -nostdin -v error -f lavfi -i 'color=c=purple:size=640x360' -frames:v 1 -threads 1 -y "$root/still.png"
printf invalid >"$root/invalid.mp4"
sha256sum "$binary" >"$root/binary.log"
timeout "$(( ${WANIENGINE_SOAK_SECONDS:-0} + 90 ))" "$binary" --verify-backend "$root" >"$root/manager.log" 2>&1 || { cat "$root/manager.log";cat "$root/shell.log" 2>/dev/null;exit 1; }
cat "$root/report.json"
