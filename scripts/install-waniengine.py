#!/usr/bin/env python3
"""Per-user WaniEngine installation. No root privileges or library deletion."""
import argparse
import ctypes.util
import hashlib
import json
import os
from pathlib import Path
import platform
import re
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


def xdg_user_dir(kind, fallback):
    """Resolve a localized XDG folder (Documents, Dokumente, Documentos...)."""
    if shutil.which('xdg-user-dir'):
        result = subprocess.run(['xdg-user-dir', kind], capture_output=True, text=True)
        if result.returncode == 0 and result.stdout.strip():
            return Path(result.stdout.strip())
    config = Path(os.environ.get('XDG_CONFIG_HOME', Path.home() / '.config'))
    dirs = config / 'user-dirs.dirs'
    if dirs.is_file():
        key = 'XDG_' + kind + '_DIR='
        for line in dirs.read_text(errors='replace').splitlines():
            line = line.strip()
            if line.startswith(key):
                value = line[len(key):].strip().strip('"').replace('$HOME', str(Path.home()))
                if os.path.isabs(value):
                    return Path(value)
    return fallback


def locations():
    home = Path.home()
    state = Path(os.environ.get('XDG_STATE_HOME', home / '.local/state'))
    data = Path(os.environ.get('XDG_DATA_HOME', home / '.local/share'))
    config = Path(os.environ.get('XDG_CONFIG_HOME', home / '.config'))
    desktop = xdg_user_dir('DESKTOP', home / 'Desktop')
    # The library lives in the user's Documents folder, matching the app's own
    # resolution. Keeping it in the state directory made every update point at
    # a location the app no longer used.
    documents = xdg_user_dir('DOCUMENTS', home / 'Documents')
    return {'home': home, 'prefix': home / '.local/opt/waniengine',
            'launcher': home / '.local/bin/waniengine', 'data': data,
            'config': config, 'desktop': desktop,
            'library': documents / 'WaniEngine',
            'state': state, 'backup': state / 'WaniEngine/installer-backups'}


def verify_bundle(bundle):
    manifest = json.loads((bundle / 'manifest.json').read_text())
    if manifest.get('appId') != APP_ID or manifest.get('architecture') != platform.machine():
        raise RuntimeError('This package does not match WaniEngine or this CPU architecture.')
    flavor = manifest.get('runtimeFlavor', 'qt')
    if flavor not in {'qt', 'egui'}:
        raise RuntimeError('This package has an unknown runtime flavor.')
    executable = manifest.get('executable', 'bin/waniengine')
    if not isinstance(executable, str) or Path(executable).is_absolute() or '..' in Path(executable).parts:
        raise RuntimeError('The package executable path is invalid.')
    manifest['runtimeFlavor'] = flavor
    manifest['executable'] = executable
    files = manifest.get('files')
    if not isinstance(files, dict):
        raise RuntimeError('The package manifest has no file list.')
    required = {executable, 'share/waniengine/waniengine.png'}
    if flavor == 'qt':
        required.update({
            'lib/qml/WaniEngine/Runtime/qmldir',
            'lib/qml/WaniEngine/Runtime/libwaniengine_runtime.so',
            'share/waniengine/wallpaper/metadata.json',
        })
    else:
        # The UI and renderer stay Rust. Only the reviewed Plasma presentation
        # adapter is allowed to contain QML; no general QML runtime is bundled.
        allowed_qml = {'share/waniengine/adapters/plasma/contents/ui/main.qml'}
        for relative in files:
            if (relative.lower().endswith('.qml') or '/qml/' in ('/' + relative.lower())) and relative not in allowed_qml:
                raise RuntimeError('Unexpected QML outside the minimal Plasma adapter: ' + relative)
        adapter = 'share/waniengine/adapters/plasma/'
        if any(relative.startswith(adapter) for relative in files):
            required.update(adapter + name for name in ['metadata.json', 'contents/config/main.xml', 'contents/ui/main.qml', 'kwin-ordering.js'])
    if not required.issubset(files):
        raise RuntimeError('The package is incomplete.')
    for relative, digest in files.items():
        path = bundle / relative
        if Path(relative).is_absolute() or '..' in Path(relative).parts or path.is_symlink() or not path.resolve().is_relative_to(bundle.resolve()):
            raise RuntimeError('Invalid package path: ' + relative)
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise RuntimeError('Package checksum mismatch: ' + relative)
    if not os.access(bundle / executable, os.X_OK):
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


def desktop_entry(executable, icon, background=False, runtime_flavor='qt'):
    # Desktop Entry Exec has its own escaping rules, distinct from shell quoting.
    quoted = '"' + str(executable).replace('\\', '\\\\').replace('"', '\\"').replace('`', '\\`').replace('$', '\\$').replace('%', '%%') + '"'
    if runtime_flavor == 'egui':
        args = ' --resume-session --background' if background else ''
    else:
        args = ' --background --resume-wallpaper' if background else ' --resume-wallpaper'
    return ('[Desktop Entry]\nType=Application\nName=WaniEngine\n'
            'Comment=Animated wallpapers and desktop characters\n'
            f'Exec={quoted}{args}\nIcon={icon}\nTerminal=false\nCategories=Graphics;\n'
            f'StartupWMClass={APP_ID}\nX-KDE-Wayland-Interfaces=org_kde_plasma_window_management\n'
            'X-KDE-autostart-phase=2\n' + MARKER + '\n')

def prepare_scene(paths, runtime_flavor='qt'):
    library = paths['library']
    legacy = paths['state'] / 'WaniEngine/waniengine/library'
    scene_path = library / 'scene.json'
    # Absorb an older state-directory library so an update never looks empty.
    if not scene_path.is_file() and legacy != library and (legacy / 'scene.json').is_file():
        library.mkdir(parents=True, exist_ok=True)
        for entry in legacy.iterdir():
            target = library / entry.name
            if target.exists():
                continue
            if entry.is_dir():
                shutil.copytree(entry, target)
            else:
                shutil.copy2(entry, target)
    scene = json.loads(scene_path.read_text()) if scene_path.is_file() else {
        'schema': 2, 'assets': [], 'actorUrls': [], 'playlist': []
    }
    if not isinstance(scene, dict) or not isinstance(scene.get('assets'), list):
        raise RuntimeError('The existing library scene is invalid; it has been left unchanged.')
    settings = scene.setdefault('settings', {})
    if not isinstance(settings, dict):
        raise RuntimeError('The existing library settings are invalid; they have been left unchanged.')
    if runtime_flavor == 'egui':
        settings.setdefault('startWithSession', False)
        settings.setdefault('restoreOnLaunch', True)
    else:
        settings.update(startWithSession=True, restoreOnLaunch=True)
    return json.dumps(scene, ensure_ascii=False, indent=2) + '\n'


def refresh_cache():
    for command in (['kbuildsycoca6', '--noincremental'],
                    ['update-desktop-database', str(locations()['data'] / 'applications')]):
        if shutil.which(command[0]):
            subprocess.run(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)


def install(bundle, paths=None, integration=True):
    paths = paths or locations()
    manifest = verify_bundle(bundle)
    flavor = manifest['runtimeFlavor']
    executable = bundle / manifest['executable']
    # Validate the current library before modifying any installed files.
    scene_text = prepare_scene(paths, flavor)
    scene = json.loads(scene_text)
    if integration:
        desktop = next((os.environ.get(name, '').strip().lower()
                        for name in ('XDG_CURRENT_DESKTOP', 'XDG_SESSION_DESKTOP', 'DESKTOP_SESSION')
                        if os.environ.get(name, '').strip()), '')
        desktops = {item.strip() for item in re.split(r'[:;,]', desktop) if item.strip()}
        is_kde = bool(desktops & {'kde', 'plasma', 'plasmawayland', 'plasmax11'})
        is_gnome = any(item == 'gnome' or item.startswith('gnome-') or item == 'ubuntu'
                       for item in desktops)
        if flavor == 'egui' and not (is_kde or is_gnome):
            raise RuntimeError('The egui package requires a KDE Plasma or GNOME session.')
        required = ['ffmpeg', 'ffprobe']
        if is_kde:
            required.append('plasmashell')
        if flavor == 'egui' and is_gnome:
            required.append('gsettings')
        for tool in required:
            if not shutil.which(tool):
                raise RuntimeError('Missing runtime dependency: ' + tool)
        if flavor == 'egui' and ctypes.util.find_library('SDL2-2.0') is None:
            raise RuntimeError('Missing runtime dependency: SDL2 audio library (libSDL2-2.0.so.0)')
        if not shutil.which('ffplay'):
            print('Character audio is unavailable until FFplay is installed. Wallpaper audio uses SDL2.')
        if is_kde and flavor == 'egui':
            print('Video wallpaper requires Plasma Wayland, layer-shell and KWin 6.5+. The app probes these capabilities before Apply.')
        checked = subprocess.run(['ldd', str(executable)], capture_output=True, text=True, timeout=10)
        if checked.returncode or 'not found' in checked.stdout:
            raise RuntimeError('Missing native runtime library:\n' + checked.stdout)
        if flavor == 'egui' and re.search(r'lib(?:Qt|qml)', checked.stdout, re.IGNORECASE):
            raise RuntimeError('The egui executable links Qt/QML libraries.')
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
                    'export WANIENGINE_DATA_ROOT=' + shlex.quote(str(paths['library'])) + '\n'
                    'export WANIENGINE_INSTALLED_SCENE=' + shlex.quote(str(state_file)) + '\n'
                    'exec ' + shlex.quote(str(current / manifest['executable'])) + ' "$@"\n')
        write_atomic(paths['launcher'], launcher, 0o755)
        icon = current / 'share/waniengine/waniengine.png'
        write_atomic(app, desktop_entry(paths['launcher'], icon, runtime_flavor=flavor))
        wants_autostart = flavor == 'qt' or scene.get('settings', {}).get('startWithSession') is True
        if wants_autostart:
            write_atomic(startup, desktop_entry(paths['launcher'], icon, True, flavor))
        elif startup.is_file() and MARKER in startup.read_text(errors='replace'):
            startup.unlink()
        write_atomic(shortcut, desktop_entry(paths['launcher'], icon, runtime_flavor=flavor), 0o755)
        write_atomic(state_file, scene_text)
        record = {'version': manifest['version'], 'runtimeFlavor': flavor,
                  'current': str(version), 'backup': str(backup),
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
    # Keep the tiny adapter when a crash recovery is pending; removing its
    # files then would remove the fallback before the desktop is restored.
    adapter = paths['data'] / 'plasma/wallpapers/org.waniengine.rustbridge'
    recovery = paths['state'] / 'WaniEngine/plasma-rust-bridge'
    adapter_marker = adapter / '.waniengine-managed'
    if adapter_marker.is_file() and adapter_marker.read_text().strip() == APP_ID and not list(recovery.glob('session-*.json')):
        shutil.rmtree(adapter)
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
