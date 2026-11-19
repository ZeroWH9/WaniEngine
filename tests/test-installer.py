#!/usr/bin/env python3
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

script = Path(__file__).resolve().parents[1] / 'scripts/install-waniengine.py'
spec = importlib.util.spec_from_file_location('installer', script)
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallationTests(unittest.TestCase):
    def setUp(self):
        self.sdl = patch.object(installer.ctypes.util,'find_library',return_value='libSDL2-2.0.so.0')
        self.sdl.start()
        self.addCleanup(self.sdl.stop)
        self.temp = tempfile.TemporaryDirectory(prefix='wani-install-test-')
        self.root = Path(self.temp.name)
        self.paths = {key: self.root / key for key in ('home', 'prefix', 'launcher', 'data', 'config', 'desktop', 'library', 'state', 'backup')}
        self.paths['launcher'] = self.root / 'bin/waniengine'
        self.bundle = self.root / 'bundle'
        self.make_bundle('0.2.0')
        self.startup = self.paths['config'] / 'autostart' / (installer.APP_ID + '.desktop')
        self.startup.parent.mkdir(parents=True)
        self.startup.write_text('[Desktop Entry]\nName=WaniEngine\n')
        self.unrelated = self.startup.parent / 'Discord.desktop'
        self.unrelated.write_text('unrelated startup\n')
        self.source = self.paths['library'] / 'scene.json'
        self.source.parent.mkdir(parents=True)
        self.source.write_text(json.dumps({'assets': [{'url': 'file:///existing/media.mp4'}], 'settings': {}}))

    def tearDown(self):
        self.temp.cleanup()

    def make_bundle(self, version, flavor='qt'):
        shutil.rmtree(self.bundle, ignore_errors=True)
        executable = 'bin/waniengine' if flavor == 'qt' else 'bin/waniengine-egui'
        files = [executable, 'share/waniengine/waniengine.png']
        manifest = {'appId': installer.APP_ID, 'version': version,
                    'architecture': platform.machine()}
        if flavor == 'qt':
            files.extend((
                'lib/qml/WaniEngine/Runtime/qmldir',
                'lib/qml/WaniEngine/Runtime/libwaniengine_runtime.so',
                'share/waniengine/wallpaper/metadata.json',
            ))
        else:
            files.extend('share/waniengine/adapters/plasma/'+name for name in ['metadata.json','contents/config/main.xml','contents/ui/main.qml','kwin-ordering.js'])
            manifest.update(runtimeFlavor='egui', executable=executable,
                            runtime='Rust/egui; FFmpeg; KDE Plasma session')
        for name in files:
            path = self.bundle / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((version + name).encode())
        (self.bundle / executable).chmod(0o755)
        manifest['files'] = {
            name: hashlib.sha256((self.bundle / name).read_bytes()).hexdigest()
            for name in files
        }
        (self.bundle / 'manifest.json').write_text(json.dumps(manifest))

    def test_adapter_is_complete_and_qml_whitelist_is_enforced(self):
        self.make_bundle('0.3.0',flavor='egui')
        self.assertEqual(installer.verify_bundle(self.bundle)['runtimeFlavor'],'egui')
        manifest=json.loads((self.bundle/'manifest.json').read_text())
        key='share/waniengine/adapters/plasma/kwin-ordering.js'
        del manifest['files'][key]
        (self.bundle/'manifest.json').write_text(json.dumps(manifest))
        with self.assertRaisesRegex(RuntimeError,'incomplete'):
            installer.verify_bundle(self.bundle)
        self.make_bundle('0.3.0',flavor='egui')
        manifest=json.loads((self.bundle/'manifest.json').read_text());manifest['files']['share/unexpected.qml']='0'*64
        (self.bundle/'manifest.json').write_text(json.dumps(manifest))
        with self.assertRaisesRegex(RuntimeError,'Unexpected QML'):
            installer.verify_bundle(self.bundle)

    def test_uninstall_preserves_adapter_when_recovery_is_pending(self):
        self.make_bundle('0.3.0',flavor='egui');installer.install(self.bundle,self.paths,integration=False)
        adapter=self.paths['data']/'plasma/wallpapers/org.waniengine.rustbridge'
        adapter.mkdir(parents=True);(adapter/'.waniengine-managed').write_text(installer.APP_ID)
        recovery=self.paths['state']/'WaniEngine/plasma-rust-bridge';recovery.mkdir(parents=True);(recovery/'session-1.json').write_text('{}')
        installer.uninstall(self.paths,integration=False)
        self.assertTrue(adapter.is_dir());self.assertTrue(self.source.exists())

    def test_install_update_and_uninstall_preserve_library_and_unrelated_startup(self):
        first = installer.install(self.bundle, self.paths, integration=False)
        self.assertIn('Name=WaniEngine', self.startup.read_text())
        own = self.paths['library'] / 'scene.json'
        self.assertEqual(json.loads(own.read_text())['assets'][0]['url'], 'file:///existing/media.mp4')
        scene = json.loads(own.read_text())
        scene['assets'].append({'url': 'file:///new/media.png'})
        own.write_text(json.dumps(scene))
        self.make_bundle('0.2.1')
        second = installer.install(self.bundle, self.paths, integration=False)
        self.assertNotEqual(first['current'], second['current'])
        self.assertEqual(len(json.loads(own.read_text())['assets']), 2)
        installer.uninstall(self.paths, integration=False)
        self.assertTrue(own.exists())
        self.assertEqual(json.loads(self.source.read_text())['assets'][1]['url'], 'file:///new/media.png')
        self.assertEqual(self.unrelated.read_text(), 'unrelated startup\n')
        self.assertFalse(self.paths['launcher'].exists())

    def test_egui_bundle_installs_without_qt_runtime_and_preserves_unmanaged_autostart(self):
        unmanaged_startup = self.startup.read_text()
        self.make_bundle('0.2.2', flavor='egui')
        manifest = installer.verify_bundle(self.bundle)
        self.assertEqual(manifest['runtimeFlavor'], 'egui')
        self.assertNotIn('lib/qml/WaniEngine/Runtime/qmldir', manifest['files'])
        result = installer.install(self.bundle, self.paths, integration=False)
        current = self.paths['prefix'] / 'current'
        self.assertIn('bin/waniengine-egui', (self.paths['launcher']).read_text())
        self.assertFalse((current / 'lib/qml').exists())
        self.assertEqual(self.startup.read_text(), unmanaged_startup)
        scene = json.loads((self.paths['library'] / 'scene.json').read_text())
        self.assertFalse(scene['settings']['startWithSession'])
        self.assertTrue(scene['settings']['restoreOnLaunch'])
        self.assertEqual(result['runtimeFlavor'], 'egui')
        installer.uninstall(self.paths, integration=False)
        self.assertEqual(self.startup.read_text(), unmanaged_startup)

    def test_egui_installer_accepts_gnome_with_gsettings(self):
        self.make_bundle('0.2.3', flavor='egui')
        available = {'ffmpeg', 'ffprobe', 'gsettings'}
        fake_which = lambda name: '/usr/bin/' + name if name in available else None
        fake_run = lambda *args, **kwargs: SimpleNamespace(returncode=0, stdout='libc.so.6')
        with patch.dict(os.environ, {
            'XDG_CURRENT_DESKTOP': 'GNOME',
            'XDG_RUNTIME_DIR': str(self.root / 'runtime'),
        }), patch.object(installer.shutil, 'which', side_effect=fake_which), \
             patch.object(installer.subprocess, 'run', side_effect=fake_run):
            result = installer.install(self.bundle, self.paths, integration=True)
        self.assertEqual(result['runtimeFlavor'], 'egui')

    def test_egui_installer_requires_sdl_and_rejects_unknown_desktop(self):
        self.make_bundle('0.2.3', flavor='egui')
        with patch.dict(os.environ, {'XDG_CURRENT_DESKTOP':'GNOME'}), \
             patch.object(installer.shutil,'which',return_value='/usr/bin/tool'), \
             patch.object(installer.ctypes.util,'find_library',return_value=None):
            with self.assertRaisesRegex(RuntimeError,'SDL2'):
                installer.install(self.bundle,self.paths,integration=True)
        with patch.dict(os.environ, {'XDG_CURRENT_DESKTOP':'notkde'}):
            with self.assertRaisesRegex(RuntimeError,'KDE Plasma or GNOME'):
                installer.install(self.bundle,self.paths,integration=True)
        self.assertFalse(self.paths['prefix'].exists())

    def test_egui_installer_requires_gsettings_on_gnome(self):
        self.make_bundle('0.2.3', flavor='egui')
        available = {'ffmpeg', 'ffprobe'}
        fake_which = lambda name: '/usr/bin/' + name if name in available else None
        with patch.dict(os.environ, {
            'XDG_CURRENT_DESKTOP': 'GNOME',
            'XDG_RUNTIME_DIR': str(self.root / 'runtime'),
        }), patch.object(installer.shutil, 'which', side_effect=fake_which), \
             patch.object(installer.subprocess, 'run', return_value=SimpleNamespace(
                 returncode=0, stdout='libc.so.6')):
            with self.assertRaisesRegex(RuntimeError, 'gsettings'):
                installer.install(self.bundle, self.paths, integration=True)

    def test_qt_to_egui_update_switches_launcher_and_keeps_user_media(self):
        first = installer.install(self.bundle, self.paths, integration=False)
        original_current = first['current']
        scene = self.paths['library'] / 'scene.json'
        saved = json.loads(scene.read_text())
        saved['assets'].append({'url': 'file:///added/media.webp'})
        scene.write_text(json.dumps(saved))
        self.make_bundle('0.2.1', flavor='egui')
        second = installer.install(self.bundle, self.paths, integration=False)
        self.assertNotEqual(first['current'], second['current'])
        self.assertIn('bin/waniengine-egui', self.paths['launcher'].read_text())
        self.assertIn('--resume-session', self.startup.read_text())
        self.assertIn('--background', self.startup.read_text())
        self.assertTrue((self.paths['prefix'] / original_current).exists())
        self.assertEqual(len(json.loads(scene.read_text())['assets']), 2)
        self.assertEqual(second['runtimeFlavor'], 'egui')

    def test_corrupt_update_leaves_previous_installation_and_autostart_untouched(self):
        installer.install(self.bundle, self.paths, integration=False)
        previous = (self.paths['prefix'] / 'current').readlink()
        startup = self.paths['config'] / 'autostart' / (installer.APP_ID + '.desktop')
        content = startup.read_bytes()
        (self.bundle / 'bin/waniengine').write_bytes(b'tampered')
        with self.assertRaisesRegex(RuntimeError, 'checksum'):
            installer.install(self.bundle, self.paths, integration=False)
        self.assertEqual((self.paths['prefix'] / 'current').readlink(), previous)
        self.assertEqual(startup.read_bytes(), content)

    def test_failed_update_rolls_back_current_library_and_launchers(self):
        installer.install(self.bundle, self.paths, integration=False)
        previous = (self.paths['prefix'] / 'current').readlink()
        own = self.paths['library'] / 'scene.json'
        content = own.read_bytes()
        launcher = self.paths['launcher'].read_bytes()
        self.make_bundle('0.2.1')
        original_write = installer.write_atomic
        failed = False
        def inject(path, data, mode=0o644):
            nonlocal failed
            if path == own and not failed:
                failed = True
                raise OSError('injected disk failure')
            original_write(path, data, mode)
        with patch.object(installer, 'write_atomic', inject):
            with self.assertRaisesRegex(OSError, 'disk failure'):
                installer.install(self.bundle, self.paths, integration=False)
        self.assertEqual((self.paths['prefix'] / 'current').readlink(), previous)
        self.assertEqual(own.read_bytes(), content)
        self.assertEqual(self.paths['launcher'].read_bytes(), launcher)
        self.assertEqual(json.loads(self.source.read_text())['assets'][0]['url'], 'file:///existing/media.mp4')

    def test_invalid_scene_does_not_disable_startup(self):
        self.source.write_text('bad json')
        with self.assertRaises(ValueError):
            installer.install(self.bundle, self.paths, integration=False)
        self.assertEqual(self.startup.read_text(), '[Desktop Entry]\nName=WaniEngine\n')
        self.assertFalse(self.paths['prefix'].exists())


if __name__ == '__main__':
    unittest.main()
