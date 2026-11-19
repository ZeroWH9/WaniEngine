#!/usr/bin/env python3
"""Assemble a verified WaniEngine bundle for install-waniengine.py (Rust/egui, or legacy Qt)."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser()
flavors = parser.add_mutually_exclusive_group()
flavors.add_argument('--egui', action='store_true', help='build the Rust/egui bundle (default)')
flavors.add_argument('--legacy-qt', action='store_true', help='build the transitional Qt/QML bundle')
parser.add_argument('--skip-build', action='store_true')
parser.add_argument('--offline', action='store_true', help='build without accessing the network')
parser.add_argument('--binary', type=Path, help='package an explicitly built candidate without compiling')
parser.add_argument('--output', type=Path, default=root / 'output/waniengine')
args = parser.parse_args()
cargo = shutil.which('cargo')
if cargo is None:
    parser.error('Cargo was not found in PATH. Install Rust with rustup or add ~/.cargo/bin to PATH.')
strip = shutil.which('strip')
if strip is None:
    parser.error('GNU strip was not found in PATH. Install binutils.')

flavor = 'qt' if args.legacy_qt else 'egui'
package_name = 'waniengine' if args.legacy_qt else 'waniengine-egui'
executable = 'bin/waniengine' if args.legacy_qt else 'bin/waniengine-egui'
if not args.skip_build and args.binary is None:
    command = [cargo, 'build', '--release', '-p', package_name]
    if args.offline:
        command.append('--offline')
    subprocess.run(command, cwd=root, check=True)
metadata = json.loads(subprocess.check_output(
    [cargo, 'metadata', '--format-version', '1', '--no-deps'], cwd=root, text=True))
package = next((package for package in metadata['packages'] if package['name'] == package_name), None)
if package is None:
    parser.error(f'Cargo package {package_name!r} is not in this workspace.')
version = package['version']
binary = args.binary.resolve() if args.binary else Path(metadata['target_directory']) / 'release' / Path(executable).name
if not binary.is_file():
    parser.error(f'Executable was not found: {binary}. Build the selected package or omit --skip-build.')
args.output.mkdir(parents=True, exist_ok=True)
architecture = platform.machine()
stem = f'WaniEngine-{version}-linux-{architecture}' if flavor == 'qt' else f'WaniEngine-egui-{version}-linux-{architecture}'
bundle = args.output / stem
if bundle.exists():
    shutil.rmtree(bundle)
(bundle / 'bin').mkdir(parents=True)
shutil.copy2(binary, bundle / executable)
(bundle / 'share/waniengine').mkdir(parents=True, exist_ok=True)
shutil.copy2(root / 'assets/waniengine.png', bundle / 'share/waniengine/waniengine.png')
shutil.copy2(root / 'scripts/install-waniengine.py', bundle / 'install-waniengine.py')
licenses=bundle / 'share/licenses/waniengine'
licenses.mkdir(parents=True)
shutil.copy2(root / 'LICENSE',licenses / 'LICENSE')
if flavor == 'egui':
    shutil.copy2(root / 'crates/waniengine-egui/assets/fonts/OFL-Inter.txt',licenses / 'OFL-Inter.txt')
    shutil.copy2(root / 'crates/waniengine-egui/assets/LICENCE-Lucide.txt',licenses / 'LICENCE-Lucide.txt')
    shutil.copy2(root / 'crates/waniengine-egui/assets/fonts/atlas/LICENSE-Atlas-icons.txt',licenses / 'LICENSE-Atlas-icons.txt')

native_files = [bundle / executable]
runtime_description = 'Rust/egui, FFmpeg, SDL2 audio and optional FFplay character audio; minimal Plasma QML composition adapter; GNOME GSettings/XWayland'
if flavor == 'egui':
    shutil.copytree(root / 'crates/waniengine-egui/assets/plasma-adapter', bundle / 'share/waniengine/adapters/plasma')
if flavor == 'qt':
    runtimes = sorted(
        (root / 'target/release/build').glob('waniengine-*/out/qt-runtime'),
        key=lambda path: path.stat().st_mtime,
    )
    if not runtimes:
        parser.error('The Qt build runtime was not generated; build the waniengine package first.')
    runtime = runtimes[-1]
    shutil.copytree(runtime / 'qml', bundle / 'lib/qml')
    shutil.copytree(runtime / 'wallpaper', bundle / 'share/waniengine/wallpaper')
    native_files.extend(bundle.rglob('*.so'))
    runtime_description = 'System Qt 6/KDE Plasma 6, LayerShellQt, FFmpeg; KDE Wayland session'

for native in native_files:
    subprocess.run([strip, '--strip-unneeded', str(native)], check=True)
files = {
    str(path.relative_to(bundle)): hashlib.sha256(path.read_bytes()).hexdigest()
    for path in sorted(bundle.rglob('*')) if path.is_file()
}
manifest = {
    'appId': 'org.waniengine.WaniEngine',
    'version': version,
    'architecture': architecture,
    'runtimeFlavor': flavor,
    'executable': executable,
    'files': files,
    'runtime': runtime_description,
    'capabilityPolicy': {'plasmaVideo': 'Wayland + layer-shell + KWin >= 6.5', 'gnomeVideo': 'XWayland', 'otherDesktops': 'unsupported'},
}
(bundle / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(json.dumps({
    'runtimeFlavor': flavor,
    'bundle': str(bundle),
}, indent=2))
