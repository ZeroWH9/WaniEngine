#!/bin/sh
# Pull the latest source and rebuild/reinstall WaniEngine. Your library is kept.
set -eu
cd "$(dirname "$0")"
[ -d .git ] || { echo 'error: update.sh must run inside a git clone.' >&2; exit 1; }
git pull --ff-only
exec ./install.sh "$@"
