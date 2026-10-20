#!/usr/bin/env python3
import hashlib
import importlib.util
import json
from pathlib import Path
import platform
import tempfile
import unittest
from unittest.mock import patch

script = Path(__file__).resolve().parents[1] / 'scripts/install-waniengine.py'
spec = importlib.util.spec_from_file_location('installer', script)
installer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installer)


class InstallationTests(unittest.TestCase):
    def setUp(self):
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

    def make_bundle(self, version):
        files = ('bin/waniengine', 'lib/qml/WaniEngine/Runtime/qmldir',
                 'lib/qml/WaniEngine/Runtime/libwaniengine_runtime.so',
                 'share/waniengine/wallpaper/metadata.json', 'share/waniengine/waniengine.png')
        for name in files:
            path = self.bundle / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes((version + name).encode())
        (self.bundle / 'bin/waniengine').chmod(0o755)
        (self.bundle / 'manifest.json').write_text(json.dumps({'appId': installer.APP_ID, 'version': version,
            'architecture': platform.machine(), 'files': {name: hashlib.sha256((self.bundle / name).read_bytes()).hexdigest() for name in files}}))

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
