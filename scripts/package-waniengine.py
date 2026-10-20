#!/usr/bin/env python3
"""Build the Rust/Qt application and produce a portable per-user .run installer."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
parser.add_argument('--skip-build', action='store_true')
parser.add_argument('--offline', action='store_true', help='build without accessing the network')
parser.add_argument('--output', type=Path, default=root / 'output/waniengine')
args = parser.parse_args()
if not args.skip_build:
    command = ['cargo', 'build', '--release', '-p', 'waniengine']
    if args.offline:
        command.append('--offline')
    subprocess.run(command, cwd=root, check=True)
metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--format-version', '1', '--no-deps'], cwd=root, text=True))
version = next(package['version'] for package in metadata['packages']
               if package['name'] == 'waniengine')
binary = root / 'target/release/waniengine'
runtimes = sorted((root / 'target/release/build').glob('waniengine-*/out/qt-runtime'), key=lambda p: p.stat().st_mtime)
runtime = runtimes[-1]
args.output.mkdir(parents=True, exist_ok=True)
bundle = args.output / f'WaniEngine-{version}-linux-{platform.machine()}'
if bundle.exists():
    shutil.rmtree(bundle)
(bundle / 'bin').mkdir(parents=True)
shutil.copy2(binary, bundle / 'bin/waniengine')
shutil.copytree(runtime / 'qml', bundle / 'lib/qml')
shutil.copytree(runtime / 'wallpaper', bundle / 'share/waniengine/wallpaper')
shutil.copy2(root / 'assets/waniengine.png', bundle / 'share/waniengine/waniengine.png')
shutil.copy2(root / 'scripts/install-waniengine.py', bundle / 'install-waniengine.py')
# The system supplies Qt/KDE and multimedia; the matching custom QML module is bundled.
for native in [bundle / 'bin/waniengine', *bundle.rglob('*.so')]:
    subprocess.run(['strip', '--strip-unneeded', str(native)], check=True)
files = {str(path.relative_to(bundle)): hashlib.sha256(path.read_bytes()).hexdigest()
         for path in sorted(bundle.rglob('*')) if path.is_file()}
manifest = {'appId': 'org.waniengine.WaniEngine', 'version': version,
            'architecture': platform.machine(), 'files': files,
            'runtime': 'System Qt 6/KDE Plasma 6, LayerShellQt, FFmpeg; KDE Wayland session'}
(bundle / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
with tempfile.TemporaryDirectory() as temporary:
    archive = Path(temporary) / 'payload.tar.gz'
    with tarfile.open(archive, 'w:gz') as tar:
        tar.add(bundle, arcname='package')
    header = '''#!/bin/sh
set -eu
command -v python3 >/dev/null || { echo 'Python 3 is required.' >&2; exit 1; }
wani_tmp=$(mktemp -d "${TMPDIR:-/tmp}/waniengine-installer.XXXXXX")
trap 'rm -rf "$wani_tmp"' EXIT HUP INT TERM
wani_line=$(awk '/^__WANIENGINE_PAYLOAD__$/ {print NR + 1; exit}' "$0")
tail -n +"$wani_line" "$0" | tar -xz -C "$wani_tmp"
python3 "$wani_tmp/package/install-waniengine.py" --bundle "$wani_tmp/package" "$@"
exit 0
__WANIENGINE_PAYLOAD__
'''
    installer = args.output / f'WaniEngine-{version}-linux-{platform.machine()}.run'
    with installer.open('wb') as output:
        output.write(header.encode())
        output.write(archive.read_bytes())
    installer.chmod(0o755)
print(json.dumps({'bundle': str(bundle), 'installer': str(installer),
                  'sha256': hashlib.sha256(installer.read_bytes()).hexdigest()}, indent=2))
