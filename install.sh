#!/bin/sh
# Build WaniEngine from source and install it for the current user (no root).
#
#   ./install.sh              build and install (or update)
#   ./install.sh --uninstall  remove the program; your library is kept
#   ./install.sh --launch     start WaniEngine after installing
#   ./install.sh --yes        do not ask before installing Rust with rustup
#
# Also usable without cloning first:
#   curl -fsSL https://raw.githubusercontent.com/ZeroWH9/WaniEngine/<tag>/install.sh | sh
# which clones $WANIENGINE_REF (default: main) into a temporary directory.
set -eu

REPO_URL=${WANIENGINE_REPO:-https://github.com/ZeroWH9/WaniEngine.git}
REF=${WANIENGINE_REF:-main}
MIN_RUST=1.88

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

action=install; launch=''; assume_yes=''
for arg in "$@"; do
    case $arg in
        --uninstall) action=uninstall ;;
        --launch) launch=--launch ;;
        --yes|-y) assume_yes=1 ;;
        -h|--help) sed -n '2,11p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) die "unknown option: $arg" ;;
    esac
done

[ "$(id -u)" -ne 0 ] || die 'run this as your desktop user, without sudo.'
command -v python3 >/dev/null 2>&1 || die 'Python 3 is required (it runs the final installation step).'

ask() { # ask "question" -> true when the user agrees
    [ -n "$assume_yes" ] && return 0
    [ -r /dev/tty ] || return 1
    printf '%s [y/N] ' "$1" >/dev/tty
    read -r answer </dev/tty || return 1
    case $answer in y|Y|yes|YES) return 0 ;; *) return 1 ;; esac
}

# Locate the source tree: next to this script, or clone a pinned ref.
root=''; cleanup_dir=''
script_dir=$(cd "$(dirname "$0")" 2>/dev/null && pwd || true)
if [ -n "$script_dir" ] && [ -f "$script_dir/scripts/package-waniengine.py" ]; then
    root=$script_dir
else
    command -v git >/dev/null 2>&1 || die 'git is required to download WaniEngine.'
    cleanup_dir=$(mktemp -d "${TMPDIR:-/tmp}/waniengine-src.XXXXXX")
    trap 'rm -rf "$cleanup_dir"' EXIT HUP INT TERM
    say "Downloading WaniEngine ($REF)..."
    git clone --quiet --depth 1 --branch "$REF" "$REPO_URL" "$cleanup_dir/src"
    root=$cleanup_dir/src
fi

if [ "$action" = uninstall ]; then
    exec python3 "$root/scripts/install-waniengine.py" --uninstall
fi

# --- Rust toolchain -------------------------------------------------------
version_ge() { # version_ge have want
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" = "$2" ]
}
PATH=$HOME/.cargo/bin:$PATH
rust_ok() {
    command -v cargo >/dev/null 2>&1 && command -v rustc >/dev/null 2>&1 || return 1
    have=$(rustc --version | sed -n 's/^rustc \([0-9.]*\).*/\1/p')
    [ -n "$have" ] && version_ge "$have" "$MIN_RUST"
}
if ! rust_ok; then
    say "Rust $MIN_RUST or newer is required to build WaniEngine."
    if ask 'Install it for your user with rustup (https://rustup.rs)?'; then
        command -v curl >/dev/null 2>&1 || die 'curl is required to download rustup.'
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
        PATH=$HOME/.cargo/bin:$PATH
        rust_ok || die "Rust $MIN_RUST or newer is still not available."
    else
        die "install Rust $MIN_RUST+ (https://rustup.rs) and run this script again."
    fi
fi

# --- System packages ------------------------------------------------------
missing=''
need_tool() { command -v "$1" >/dev/null 2>&1 || missing="$missing $1"; }
need_pc() { pkg-config --exists "$1" 2>/dev/null || missing="$missing lib:$1"; }
need_tool cc
need_tool pkg-config
need_tool strip
need_tool ffmpeg
need_tool ffprobe
if command -v pkg-config >/dev/null 2>&1; then
    for pc in wayland-client xkbcommon egl gl x11 xcursor xi xrandr alsa; do need_pc "$pc"; done
fi
if [ -n "$missing" ]; then
    say "Missing build/runtime dependencies:$missing"
    if command -v pacman >/dev/null 2>&1; then
        say 'Arch:    sudo pacman -S --needed base-devel pkgconf wayland libxkbcommon mesa libx11 libxcursor libxi libxrandr alsa-lib sdl2 ffmpeg'
    elif command -v apt-get >/dev/null 2>&1; then
        say 'Debian/Ubuntu: sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev libegl1-mesa-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev libasound2-dev libsdl2-2.0-0 ffmpeg'
    elif command -v dnf >/dev/null 2>&1; then
        say 'Fedora: sudo dnf install gcc pkgconf-pkg-config wayland-devel libxkbcommon-devel mesa-libEGL-devel mesa-libGL-devel libX11-devel libXcursor-devel libXi-devel libXrandr-devel alsa-lib-devel SDL2 ffmpeg'
    fi
    die 'install the packages above and run this script again.'
fi

# --- Build and install ----------------------------------------------------
say 'Building WaniEngine (release). This can take several minutes...'
(cd "$root" && cargo build --release --locked -p waniengine-egui)
out=$(mktemp -d "${TMPDIR:-/tmp}/waniengine-bundle.XXXXXX")
if [ -n "$cleanup_dir" ]; then
    trap 'rm -rf "$cleanup_dir" "$out"' EXIT HUP INT TERM
else
    trap 'rm -rf "$out"' EXIT HUP INT TERM
fi
python3 "$root/scripts/package-waniengine.py" --egui --skip-build --output "$out" >/dev/null
bundle=$(find "$out" -mindepth 1 -maxdepth 1 -type d | head -n1)
[ -n "$bundle" ] || die 'bundle was not created.'
python3 "$bundle/install-waniengine.py" --bundle "$bundle" $launch
say 'WaniEngine is installed. Start it from your application menu.'
