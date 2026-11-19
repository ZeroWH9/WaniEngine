#!/usr/bin/env bash
# Bound a complete build/test runner and all its children, not individual rustc jobs.
set -euo pipefail
if [[ ${1:-} != --inside ]]; then
    root=$(cd -- "$(dirname -- "$0")/.." && pwd)
    exec systemd-run --user --unit="wani-check-$(date +%s)-$$" --pipe --wait --collect --setenv=PATH \
        --property=MemoryMax=3000000000 --property=MemorySwapMax=0 --property=CPUQuota=100% --property=TasksMax=256 \
        --working-directory="$root" "$root/scripts/run-limited.sh" --inside "$@"
fi
shift
cg=$(awk -F: '$1=="0" { print $3 }' /proc/self/cgroup)
limit=$(cat "/sys/fs/cgroup$cg/memory.max")
swap=$(cat "/sys/fs/cgroup$cg/memory.swap.max")
[[ "$limit" =~ ^[0-9]+$ && "$limit" -gt 0 && "$limit" -le 3000000000 && "$swap" == 0 ]] || { echo 'Required cgroup limits were not applied' >&2; exit 2; }
export CARGO_BUILD_JOBS=1
printf 'RAM limit: %s bytes; swap: %s; command: %s\n' "$limit" "$swap" "$*"
"$@"
