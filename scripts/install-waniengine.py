#!/usr/bin/env python3
"""Per-user WaniEngine installation. No root privileges or library deletion."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import socket
import subprocess
import tempfile
import time

APP_ID = 'org.waniengine.WaniEngine'
MARKER = 'X-WaniEngine-Managed=true'


def write_atomic(path, data, mode=0o644):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix='.' + path.name, dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as out:
            out.write(data)
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        Path(name).unlink(missing_ok=True)


def locations():
    home = Path.home()
    state = Path(os.environ.get('XDG_STATE_HOME', home / '.local/state'))
    data = Path(os.environ.get('XDG_DATA_HOME', home / '.local/share'))
    config = Path(os.environ.get('XDG_CONFIG_HOME', home / '.config'))
    desktop = home / 'Desktop'
    if shutil.which('xdg-user-dir'):
        result = subprocess.run(['xdg-user-dir', 'DESKTOP'], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            desktop = Path(result.stdout.strip())
    return {'home': home, 'prefix': home / '.local/opt/waniengine',
            'launcher': home / '.local/bin/waniengine', 'data': data,
            'config': config, 'desktop': desktop,
            'library': state / 'WaniEngine/waniengine/library',
            'state': state, 'backup': state / 'WaniEngine/installer-backups'}


def verify_bundle(bundle):
    manifest = json.loads((bundle / 'manifest.json').read_text())
    if manifest.get('appId') != APP_ID or manifest.get('architecture') != platform.machine():
        raise RuntimeError('This package does not match WaniEngine or this CPU architecture.')
    required = {'bin/waniengine', 'lib/qml/WaniEngine/Runtime/qmldir',
                'lib/qml/WaniEngine/Runtime/libwaniengine_runtime.so',
                'share/waniengine/wallpaper/metadata.json', 'share/waniengine/waniengine.png'}
    if not required.issubset(manifest['files']):
        raise RuntimeError('The package is incomplete.')
    for relative, digest in manifest['files'].items():
        path = bundle / relative
        if Path(relative).is_absolute() or '..' in Path(relative).parts or path.is_symlink() or not path.resolve().is_relative_to(bundle.resolve()):
            raise RuntimeError('Invalid package path: ' + relative)
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise RuntimeError('Package checksum mismatch: ' + relative)
    if not os.access(bundle / 'bin/waniengine', os.X_OK):
        raise RuntimeError('The WaniEngine executable is not executable.')
    return manifest


def stop_managers():
    runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/' + str(os.getuid())))
    path = runtime / 'waniengine-control.sock'
    if not path.exists():
        return
    try:
        with socket.socket(socket.AF_UNIX) as channel:
            channel.settimeout(3)
            channel.connect(str(path))
            channel.sendall(b'{"action":"exit"}\n')
        # Exit restores the previous wallpaper and tears down actor workers.
        for _ in range(100):
            if not path.exists():
                break
            time.sleep(0.05)
    except (OSError, TimeoutError) as error:
        raise RuntimeError('Cannot stop the running manager: ' + str(error)) from error
    # A stale socket is harmless; a responsive live process after Exit is not.
    try:
        with socket.socket(socket.AF_UNIX) as channel:
            channel.settimeout(0.2)
            channel.connect(str(path))
    except OSError:
        return
    raise RuntimeError('The running manager did not exit: ' + str(path))


def desktop_entry(executable, icon, background=False):
    # Desktop Entry Exec has its own escaping rules, distinct from shell quoting.
    quoted = '"' + str(executable).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%') + '"'
    args = ' --background --resume-wallpaper' if background else ' --resume-wallpaper'
    return ('[Desktop Entry]\nType=Application\nName=WaniEngine\n'
            'Comment=Animated wallpapers and desktop characters\n'
            f'Exec={quoted}{args}\nIcon={icon}\nTerminal=false\nCategories=Graphics;\n'
            f'StartupWMClass={APP_ID}\nX-KDE-Wayland-Interfaces=org_kde_plasma_window_management\n'
            'X-KDE-autostart-phase=2\n' + MARKER + '\n')


def prepare_scene(paths):
    scene_path = paths['library'] / 'scene.json'
    scene = json.loads(scene_path.read_text()) if scene_path.is_file() else {
        'schema': 2, 'assets': [], 'actorUrls': [], 'playlist': []
    }
    if not isinstance(scene, dict) or not isinstance(scene.get('assets'), list):
        raise RuntimeError('The existing library scene is invalid; it has been left unchanged.')
    scene.setdefault('settings', {}).update(startWithSession=True, restoreOnLaunch=True)
    return json.dumps(scene, ensure_ascii=False, indent=2) + '\n'


def refresh_cache():
    for command in (['kbuildsycoca6', '--noincremental'],
                    ['update-desktop-database', str(locations()['data'] / 'applications')]):
        if shutil.which(command[0]):
            subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)


def install(bundle, paths=None, integration=True):
    paths = paths or locations()
    manifest = verify_bundle(bundle)
    # Validate the current library before modifying any installed files.
    scene = prepare_scene(paths)
    if integration:
        is_kde = 'kde' in os.environ.get('XDG_CURRENT_DESKTOP', '').lower()
        required = ['ffmpeg', 'ffprobe']
        if is_kde:
            required.append('plasmashell')
        for tool in required:
            if not shutil.which(tool):
                raise RuntimeError('Missing runtime dependency: ' + tool)
        checked = subprocess.run(['ldd', str(bundle / 'bin/waniengine')], capture_output=True, text=True)
        if checked.returncode or 'not found' in checked.stdout:
            raise RuntimeError('Missing native Qt runtime library:\n' + checked.stdout)
        stop_managers()
    prefix = paths['prefix']
    prefix.mkdir(parents=True, exist_ok=True)
    if prefix.is_symlink():
        raise RuntimeError('The installation prefix must not be a symlink.')
    marker = prefix / '.waniengine-installation'
    if any(prefix.iterdir()) and not marker.exists():
        raise RuntimeError('The installation directory contains unrelated files.')
    write_atomic(marker, APP_ID + '\n')
    version_key = manifest['version'] + '-' + hashlib.sha256((bundle / 'manifest.json').read_bytes()).hexdigest()[:12]
    if '/' in version_key or '..' in version_key:
        raise RuntimeError('Invalid package version.')
    version = prefix / 'versions' / version_key
    version.parent.mkdir(exist_ok=True)
    if not version.exists():
        stage = Path(tempfile.mkdtemp(prefix='.stage-', dir=version.parent))
        try:
            shutil.copytree(bundle, stage, dirs_exist_ok=True, symlinks=False)
            verify_bundle(stage)
            os.replace(stage, version)
        finally:
            if stage.exists():
                shutil.rmtree(stage)
    else:
        verify_bundle(version)
    current = prefix / 'current'
    if current.exists() and not current.is_symlink():
        raise RuntimeError('The current installation link is not a symlink.')
    # Preserve overwritten launchers/state for recovery, including prior updates.
    backup = paths['backup'] / time.strftime('%Y%m%d-%H%M%S')
    backup.mkdir(parents=True, exist_ok=True)
    app = paths['data'] / 'applications' / (APP_ID + '.desktop')
    startup = paths['config'] / 'autostart' / (APP_ID + '.desktop')
    shortcut = paths['desktop'] / (APP_ID + '.desktop')
    state_file = paths['library'] / 'scene.json'
    mutations = [paths['launcher'], app, startup, shortcut, state_file]
    old = {path: (path.read_bytes(), path.stat().st_mode & 0o777) if path.is_file() else None for path in mutations}
    previous = os.readlink(current) if current.is_symlink() else None
    try:
        for index, (path, saved) in enumerate(old.items()):
            if saved:
                (backup / f'{index}-{path.name}').write_bytes(saved[0])
        if previous:
            write_atomic(backup / 'previous-version.txt', previous + '\n')
        temporary = prefix / '.current-next'
        temporary.unlink(missing_ok=True)
        temporary.symlink_to(version.relative_to(prefix))
        os.replace(temporary, current)
        launcher = ('#!/bin/sh\n# WaniEngine managed launcher\n'
                    'export WANIENGINE_LAUNCHER=' + shlex.quote(str(paths['launcher'])) + '\n'
                    'exec ' + shlex.quote(str(current / 'bin/waniengine')) + ' "$@"\n')
        write_atomic(paths['launcher'], launcher, 0o755)
        icon = current / 'share/waniengine/waniengine.png'
        write_atomic(app, desktop_entry(paths['launcher'], icon))
        write_atomic(startup, desktop_entry(paths['launcher'], icon, True))
        write_atomic(shortcut, desktop_entry(paths['launcher'], icon), 0o755)
        write_atomic(state_file, scene)
        record = {'version': manifest['version'], 'current': str(version), 'backup': str(backup),
                  'library': str(paths['library'])}
        write_atomic(prefix / 'installation.json', json.dumps(record, indent=2) + '\n')
    except Exception:
        for path, saved in old.items():
            if saved:
                write_atomic(path, saved[0].decode(), saved[1])
            else:
                path.unlink(missing_ok=True)
        current.unlink(missing_ok=True)
        if previous:
            current.symlink_to(previous)
        raise
    if integration:
        refresh_cache()
        # KDE also sees the PNG through its conventional themed icon location.
        icon_target = paths['data'] / 'icons/hicolor/1024x1024/apps/waniengine.png'
        icon_target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(icon, icon_target)
        if shutil.which('gtk-update-icon-cache'):
            subprocess.run(['gtk-update-icon-cache', '-f', '-t', str(paths['data'] / 'icons/hicolor')], capture_output=True)
        if shutil.which('gio'):
            subprocess.run(['gio', 'set', str(shortcut), 'metadata::trusted', 'true'], capture_output=True)
    return record


def uninstall(paths=None, integration=True):
    paths = paths or locations()
    prefix = paths['prefix']
    if not (prefix / '.waniengine-installation').is_file():
        raise RuntimeError('No managed WaniEngine installation was found.')
    if integration:
        stop_managers()
    for path in (paths['data'] / 'applications' / (APP_ID + '.desktop'),
                 paths['config'] / 'autostart' / (APP_ID + '.desktop'),
                 paths['desktop'] / (APP_ID + '.desktop')):
        if path.is_file() and MARKER in path.read_text():
            path.unlink()
    launcher = paths['launcher']
    if launcher.is_file() and '# WaniEngine managed launcher' in launcher.read_text():
        launcher.unlink()
    shutil.rmtree(prefix)
    if integration:
        refresh_cache()
    return {'uninstalled': True, 'preservedLibrary': str(paths['library']), 'backups': str(paths['backup'])}


def main():
    parser = argparse.ArgumentParser(description='Install or update WaniEngine for the current Linux user.')
    parser.add_argument('--bundle', type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument('--uninstall', action='store_true')
    parser.add_argument('--launch', action='store_true')
    args = parser.parse_args()
    if os.getuid() == 0:
        parser.error('Run this installer as your desktop user, without sudo.')
    try:
        record = uninstall() if args.uninstall else install(args.bundle.resolve())
        print(json.dumps(record, indent=2))
        if args.launch and not args.uninstall:
            log = locations()['library'].parent / 'waniengine.log'
            with log.open('ab') as output:
                subprocess.Popen([str(locations()['launcher']), '--resume-wallpaper'], stdout=output, stderr=output, start_new_session=True)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        parser.exit(1, 'WaniEngine installation failed: ' + str(error) + '\n')


if __name__ == '__main__':
    main()
