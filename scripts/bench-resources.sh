#!/usr/bin/env bash
# Measures WaniEngine resident memory while a wallpaper runs, and reports VRAM
# only when the driver can attribute it reliably.
#
# Usage:
#   BENCH_LIBRARY=<library-dir> scripts/bench-resources.sh [seconds]
#
# The library must already contain the wallpaper under test.
#
# VRAM caveat: on Wayland with the NVIDIA driver, nvidia-smi does not list
# per-process memory for this application, and the global total is too noisy
# (other desktop apps allocate and free memory during the run). When that
# happens this script reports "unavailable" instead of a misleading number.
set -euo pipefail

SECONDS_TO_RUN=${1:-20}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
BINARY="${WANIENGINE_BINARY:-$ROOT/target/debug/waniengine}"

[[ -x "$BINARY" ]] || { echo "build first: cargo build -p waniengine" >&2; exit 1; }
[[ -n "${BENCH_LIBRARY:-}" ]] || { echo "set BENCH_LIBRARY to a prepared library" >&2; exit 1; }

have_nvidia=0
command -v nvidia-smi >/dev/null 2>&1 && have_nvidia=1

gpu_total() {
    [[ $have_nvidia -eq 1 ]] || { echo 0; return; }
    nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits | head -1 | tr -d ' '
}

gpu_for_pid() {
    local pid=$1
    [[ $have_nvidia -eq 1 ]] || { echo ""; return; }
    nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader,nounits \
        | tr -d ' ' | awk -F',' -v p="$pid" '$1 == p {print $2}' | head -1
}

rss_for_pid() {
    awk '/VmRSS/ {print int($2/1024)}' "/proc/$1/status" 2>/dev/null || echo 0
}

work=$(mktemp -d)
cleanup() { pkill -x waniengine 2>/dev/null || true; pkill -x "$(basename "$BINARY")" 2>/dev/null || true; rm -rf "$work"; }
trap cleanup EXIT

pkill -x waniengine 2>/dev/null || true
sleep 2

HOME="$work" XDG_CURRENT_DESKTOP=GNOME "$BINARY" --data-dir "$BENCH_LIBRARY" >"$work/log" 2>&1 &
app_pid=$!
sleep 8

if ! kill -0 "$app_pid" 2>/dev/null; then
    echo "waniengine did not stay running; log:" >&2
    tail -20 "$work/log" >&2
    exit 1
fi

rss_peak=0
gpu_peak=""
deadline=$((SECONDS + SECONDS_TO_RUN))
while (( SECONDS < deadline )); do
    rss=$(rss_for_pid "$app_pid")
    (( rss > rss_peak )) && rss_peak=$rss
    gpu=$(gpu_for_pid "$app_pid")
    if [[ -n "$gpu" ]]; then
        [[ -z "$gpu_peak" ]] && gpu_peak=0
        (( gpu > gpu_peak )) && gpu_peak=$gpu
    fi
    sleep 1
done

echo "pid:        $app_pid"
echo "peak RSS:   ${rss_peak} MiB"
if [[ -n "$gpu_peak" ]]; then
    echo "peak VRAM:  ${gpu_peak} MiB"
else
    echo "peak VRAM:  unavailable (driver does not attribute memory to this process)"
fi
