#!/usr/bin/env bash
set -euo pipefail
rust_root=$(realpath "$(dirname "$0")/..")
binary="$rust_root/target/debug/waniengine"
cargo build --offline --manifest-path "$rust_root/Cargo.toml" -p waniengine
# KDE requires a matching desktop identity to expose fullscreen/maximized state.
# Register the parallel Rust application without replacing the installed manager.
python3 - "$rust_root" "$binary" <<'PY'
import os
import pathlib
import sys
root, binary = map(pathlib.Path, sys.argv[1:])
data = pathlib.Path(os.environ.get('XDG_DATA_HOME', pathlib.Path.home() / '.local/share'))
target = data / 'applications/org.waniengine.WaniEngine.desktop'
target.parent.mkdir(parents=True, exist_ok=True)
executable = str(binary).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%')
text = (root / 'packaging/org.waniengine.WaniEngine.desktop').read_text()
text = text.replace('Exec=waniengine --resume-wallpaper', f'Exec="{executable}" --redesign')
icon = root / 'assets/waniengine.png'
text = text.replace('Icon=waniengine', f'Icon={icon}')
temporary = target.with_suffix('.desktop.tmp')
temporary.write_text(text)
temporary.replace(target)
PY
kbuildsycoca6 --noincremental
exec "$binary" --redesign "$@"
