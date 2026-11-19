#!/usr/bin/env bash
# Run the experimental layer renderer in an isolated two-output KWin session.
set -euo pipefail
if [[ ${1:-} != --inside ]]; then
    project_root=$(realpath "$(dirname "$0")/..")
    cargo build --offline --manifest-path "$project_root/Cargo.toml" -p waniengine-wayland --bin waniengine-wayland --example test_pointer
    cargo build --offline --manifest-path "$project_root/Cargo.toml" -p waniengine-egui
    test_root=$(mktemp -d /tmp/waniengine-wayland-test.XXXXXX)
    mkdir -p "$test_root"/{runtime,config,data,cache,state}
    chmod 700 "$test_root/runtime"
    printf '#!/usr/bin/env bash\nexec %q --overlay-worker "$@"\n' "$project_root/target/debug/waniengine-egui" > "$test_root/embedded-worker"
    chmod +x "$test_root/embedded-worker"
    export XDG_RUNTIME_DIR="$test_root/runtime" XDG_CONFIG_HOME="$test_root/config" XDG_DATA_HOME="$test_root/data"
    export XDG_CACHE_HOME="$test_root/cache" XDG_STATE_HOME="$test_root/state"
    unset DISPLAY WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS SESSION_MANAGER
    exec dbus-run-session -- bash "$0" --inside "$test_root" "$test_root/embedded-worker" "$project_root/target/debug/examples/test_pointer"
fi

test_root=$2
runtime_binary=$3
pointer_binary=$4
[[ "$XDG_RUNTIME_DIR" == "$test_root/runtime" && "$test_root" == /tmp/waniengine-wayland-test.* ]]
export XDG_CURRENT_DESKTOP=KDE XDG_SESSION_TYPE=wayland LIBGL_ALWAYS_SOFTWARE=1
kwin_pid='' layer_pid='' actor_pid='' clickdot_pid=''
cleanup() {
    for child in "$clickdot_pid" "$actor_pid" "$layer_pid" "$kwin_pid"; do
        [[ -z "$child" ]] || kill "$child" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT
QT_QPA_PLATFORM=offscreen kwin_wayland --virtual --socket wani-runtime --width 800 --height 600 --output-count 2 --no-global-shortcuts --no-lockscreen --no-kactivities >"$test_root/kwin.log" 2>&1 &
kwin_pid=$!
for ((attempt=0; attempt<100; attempt++)); do
    [[ -S "$XDG_RUNTIME_DIR/wani-runtime" ]] && break
    kill -0 "$kwin_pid" || { cat "$test_root/kwin.log"; exit 1; }
    sleep 0.1
done
[[ -S "$XDG_RUNTIME_DIR/wani-runtime" ]]
export WAYLAND_DISPLAY=wani-runtime QT_QPA_PLATFORM=wayland
dbus-update-activation-environment WAYLAND_DISPLAY QT_QPA_PLATFORM XDG_CURRENT_DESKTOP XDG_SESSION_TYPE
"$runtime_binary" --list-outputs >"$test_root/outputs.json"
output=$(python3 - "$test_root/outputs.json" <<'PY'
import json, sys
outputs = json.load(open(sys.argv[1]))
assert len(outputs) == 2, outputs
assert all(item['name'] and item['logicalPosition'] is not None for item in outputs), outputs
print(max(outputs, key=lambda item: item['logicalPosition'][0])['name'])
PY
)
python3 - "$test_root/wallpaper.png" <<'PY'
import struct, sys, zlib
width, height = 64, 64
def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
rows = b''.join(b'\0' + bytes([255, 30, 35]) * width for _ in range(height))
with open(sys.argv[1], 'wb') as image:
    image.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))
PY
if "$runtime_binary" --image "$test_root/wallpaper.png" --output nonexistent >"$test_root/missing.out" 2>"$test_root/missing.err"; then
    echo 'Unknown output was incorrectly accepted' >&2
    exit 1
fi
"$runtime_binary" --image "$test_root/wallpaper.png" --output "$output" --layer bottom >"$test_root/layer.out" 2>"$test_root/layer.err" &
layer_pid=$!
for ((attempt=0; attempt<100; attempt++)); do
    [[ -s "$test_root/layer.out" ]] && break
    kill -0 "$layer_pid" || { cat "$test_root/layer.err"; exit 1; }
    sleep 0.1
done
python3 - "$test_root/layer.out" "$output" <<'PY'
import json, sys
message = json.loads(open(sys.argv[1]).readline())
assert message['event'] == 'configured' and message['output'] == sys.argv[2], message
assert (message['width'], message['height']) == (800, 600), message
PY
# Capture compositor output and verify both that the selected output is red and
# that the other one was not changed. Spectacle and FFmpeg are test-only tools.
timeout 20 spectacle -b -n -o "$test_root/screen.png" >"$test_root/spectacle.log" 2>&1 || { cat "$test_root/spectacle.log"; exit 1; }
ffmpeg -nostdin -v error -i "$test_root/screen.png" -f rawvideo -pix_fmt rgb24 -y "$test_root/screen.rgb"
python3 - "$test_root/screen.rgb" <<'PY'
import sys
pixels = open(sys.argv[1], 'rb').read()
assert len(pixels) == 1600 * 600 * 3, len(pixels)
def sample(x, y):
    offset = (y * 1600 + x) * 3
    return tuple(pixels[offset:offset+3])
left, right = sample(400, 300), sample(1200, 300)
assert left == (0, 0, 0), left
assert right[0] > 220 and right[1] < 60 and right[2] < 60, right
PY
kill -0 "$layer_pid"
python3 - "$test_root/actor.png" <<'PY'
import struct, sys, zlib
width, height = 64, 64
def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
rows = []
for y in range(height):
    row = bytearray()
    for x in range(width):
        row.extend((0, 255, 0, 255) if 8 <= x < 56 and 8 <= y < 56 else (0, 0, 0, 0))
    rows.append(b'\0' + row)
with open(sys.argv[1], 'wb') as image:
    image.write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b''.join(rows))) + chunk(b'IEND', b''))
PY
"$runtime_binary" --actor --image "$test_root/actor.png" --output "$output" --x 0.5 --y 0.5 --scale 200 --opacity 50 >"$test_root/actor.out" 2>"$test_root/actor.err" &
actor_pid=$!
for ((attempt=0; attempt<100; attempt++)); do
    [[ -s "$test_root/actor.out" ]] && break
    kill -0 "$actor_pid" || { cat "$test_root/actor.err"; exit 1; }
    sleep 0.1
done
python3 - "$test_root/actor.out" "$output" <<'PY'
import json, sys
message = json.loads(open(sys.argv[1]).readline())
assert message['event'] == 'configured' and message['output'] == sys.argv[2], message
assert (message['width'], message['height']) == (128, 128), message
PY
# The actor is partly transparent; both the blended center and transparent edge
# must preserve the underlying red wallpaper.
timeout 20 spectacle -b -n -o "$test_root/overlay.png" >"$test_root/overlay-capture.log" 2>&1 || { cat "$test_root/overlay-capture.log"; exit 1; }
ffmpeg -nostdin -v error -i "$test_root/overlay.png" -f rawvideo -pix_fmt rgb24 -y "$test_root/overlay.rgb"
python3 - "$test_root/overlay.rgb" <<'PY'
import sys
pixels = open(sys.argv[1], 'rb').read()
def sample(x, y):
    offset = (y * 1600 + x) * 3
    return tuple(pixels[offset:offset+3])
center, edge, left = sample(1200, 240), sample(1138, 174), sample(400, 300)
assert 110 <= center[0] <= 145 and 125 <= center[1] <= 160 and center[2] < 35, center
assert edge[0] > 220 and edge[1] < 60 and edge[2] < 60, edge
assert left == (0, 0, 0), left
PY
kill -0 "$actor_pid"
kill "$actor_pid"
wait "$actor_pid" 2>/dev/null || true
actor_pid=''
WANIENGINE_OVERLAY_INTEGRATION=1 WANIENGINE_TEST_ACTOR="$test_root/actor.png" WANIENGINE_TEST_OUTPUT="$output" \
    cargo test --offline --manifest-path "$(dirname "$0")/../Cargo.toml" -p waniengine-egui --test overlay_controller -- --ignored --nocapture
# Two slow GIF frames let the compositor and screenshot client settle before
# sampling each color. The process must stop on the last frame when not looping.
ffmpeg -nostdin -v error \
    -f lavfi -i 'color=c=0x00ff00:s=64x64:r=1/4:d=4' \
    -f lavfi -i 'color=c=blue:s=64x64:r=1/4:d=4' \
    -filter_complex '[0:v][1:v]concat=n=2:v=1:a=0[v]' -map '[v]' -loop 0 -y "$test_root/actor.gif"
"$runtime_binary" --actor --image "$test_root/actor.gif" --output "$output" --x 0.5 --y 0.5 --scale 200 --no-loop >"$test_root/gif.out" 2>"$test_root/gif.err" &
actor_pid=$!
for ((attempt=0; attempt<100; attempt++)); do
    [[ -s "$test_root/gif.out" ]] && break
    kill -0 "$actor_pid" || { cat "$test_root/gif.err"; exit 1; }
    sleep 0.1
done
timeout 20 spectacle -b -n -o "$test_root/gif-first.png" >"$test_root/gif-first.log" 2>&1 || { cat "$test_root/gif-first.log"; exit 1; }
sleep 4.5
timeout 20 spectacle -b -n -o "$test_root/gif-last.png" >"$test_root/gif-last.log" 2>&1 || { cat "$test_root/gif-last.log"; exit 1; }
sleep 4
timeout 20 spectacle -b -n -o "$test_root/gif-stopped.png" >"$test_root/gif-stopped.log" 2>&1 || { cat "$test_root/gif-stopped.log"; exit 1; }
for frame in first last stopped; do
    ffmpeg -nostdin -v error -i "$test_root/gif-$frame.png" -f rawvideo -pix_fmt rgb24 -y "$test_root/gif-$frame.rgb"
done
python3 - "$test_root/gif-first.rgb" "$test_root/gif-last.rgb" "$test_root/gif-stopped.rgb" <<'PY'
import sys
def sample(path):
    pixels = open(path, 'rb').read()
    offset = (240 * 1600 + 1200) * 3
    return tuple(pixels[offset:offset+3])
first, last, stopped = sample(sys.argv[1]), sample(sys.argv[2]), sample(sys.argv[3])
assert first[1] > 200 and first[0] < 50 and first[2] < 50, first
assert last[2] > 200 and last[0] < 50 and last[1] < 50, last
assert stopped[2] > 200 and stopped[0] < 50 and stopped[1] < 50, stopped
PY
kill -0 "$actor_pid"
kill "$actor_pid"
wait "$actor_pid" 2>/dev/null || true
actor_pid=''
# Exercise the same playback path for APNG and animated WebP when the local
# FFmpeg build can generate those test fixtures.
ffmpeg -hide_banner -encoders >"$test_root/encoders.txt" 2>/dev/null
for format in apng webp; do
    if [[ "$format" == apng ]]; then
        codec=apng
        destination="$test_root/animated.png"
        extra=(-plays 0 -f apng)
    else
        codec=libwebp_anim
        destination="$test_root/animated.webp"
        extra=(-loop 0)
    fi
    if ! rg -q " $codec " "$test_root/encoders.txt"; then
        printf 'Skipping %s fixture: encoder unavailable\n' "$format"
        continue
    fi
    ffmpeg -nostdin -v error \
        -f lavfi -i 'color=c=0x00ff00:s=64x64:r=1/4:d=4' \
        -f lavfi -i 'color=c=blue:s=64x64:r=1/4:d=4' \
        -filter_complex '[0:v][1:v]concat=n=2:v=1:a=0[v]' -map '[v]' -c:v "$codec" "${extra[@]}" -y "$destination"
    "$runtime_binary" --actor --image "$destination" --output "$output" --x 0.5 --y 0.5 --scale 200 --speed 300 --no-loop >"$test_root/$format.out" 2>"$test_root/$format.err" &
    actor_pid=$!
    for ((attempt=0; attempt<100; attempt++)); do
        [[ -s "$test_root/$format.out" ]] && break
        kill -0 "$actor_pid" || { cat "$test_root/$format.err"; exit 1; }
        sleep 0.1
    done
    sleep 3
    timeout 20 spectacle -b -n -o "$test_root/$format.png" >"$test_root/$format-capture.log" 2>&1 || { cat "$test_root/$format-capture.log"; exit 1; }
    ffmpeg -nostdin -v error -i "$test_root/$format.png" -f rawvideo -pix_fmt rgb24 -y "$test_root/$format.rgb"
    python3 - "$test_root/$format.rgb" "$format" <<'PY'
import sys
pixels = open(sys.argv[1], 'rb').read()
offset = (240 * 1600 + 1200) * 3
color = tuple(pixels[offset:offset+3])
assert color[2] > 200 and color[0] < 50 and color[1] < 50, (sys.argv[2], color)
PY
    kill -0 "$actor_pid"
    kill "$actor_pid"
    wait "$actor_pid" 2>/dev/null || true
    actor_pid=''
done
python3 "$(dirname "$0")/test-wayland-control.py" "$test_root" "$runtime_binary" "$output"
# Verify input delivery, not just the protocol request: place an opaque actor
# over a normal window and inject a click in the isolated compositor.
WAYLAND_DEBUG=client weston-clickdot >"$test_root/clickdot.out" 2>"$test_root/clickdot.log" &
clickdot_pid=$!
sleep 2
kill -0 "$clickdot_pid"
first_output=$(python3 - "$test_root/outputs.json" <<'PY'
import json, sys
outputs = json.load(open(sys.argv[1]))
print(min(outputs, key=lambda item: item['logicalPosition'][0])['name'])
PY
)
"$runtime_binary" --actor --image "$test_root/wallpaper.png" --output "$first_output" --x 0.5 --y 0.5 --scale 200 >"$test_root/click-actor.out" 2>"$test_root/click-actor.err" &
actor_pid=$!
for ((attempt=0; attempt<100; attempt++)); do
    [[ -s "$test_root/click-actor.out" ]] && break
    kill -0 "$actor_pid" || { cat "$test_root/click-actor.err"; exit 1; }
    sleep 0.1
done
timeout 20 spectacle -b -n -o "$test_root/click-overlay.png" >"$test_root/click-capture.log" 2>&1 || { cat "$test_root/click-capture.log"; exit 1; }
ffmpeg -nostdin -v error -i "$test_root/click-overlay.png" -f rawvideo -pix_fmt rgb24 -y "$test_root/click-overlay.rgb"
python3 - "$test_root/click-overlay.rgb" <<'PY'
import sys
pixels = open(sys.argv[1], 'rb').read()
offset = (240 * 1600 + 400) * 3
color = tuple(pixels[offset:offset+3])
assert color[0] > 220 and color[1] < 60 and color[2] < 60, color
PY
"$pointer_binary" 400 240
for ((attempt=0; attempt<30; attempt++)); do
    rg -q 'wl_pointer#[0-9]+\.button\([^,]+, [^,]+, 272, 0\)' "$test_root/clickdot.log" && break
    sleep 0.1
done
rg -q 'wl_pointer#[0-9]+\.button\([^,]+, [^,]+, 272, 1\)' "$test_root/clickdot.log"
rg -q 'wl_pointer#[0-9]+\.button\([^,]+, [^,]+, 272, 0\)' "$test_root/clickdot.log"
printf 'Isolated Wayland runtime test passed. Logs: %s\n' "$test_root"
