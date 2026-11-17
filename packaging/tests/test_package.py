import hashlib
import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
VENDOR = Path(os.environ.get('SKJALF_TEST_MPV_LIB_DIR', ROOT / '.deps/mpv/usr/lib/x86_64-linux-gnu'))
BINARY = Path(os.environ.get('SKJALF_TEST_BINARY', ROOT / 'target/debug/skjalf'))

spec = importlib.util.spec_from_file_location('shortcuts', ROOT / 'packaging/update-shortcuts.py')
shortcuts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcuts)

LOCK = ROOT / 'packaging/steamos-runtime.lock'
FETCH = ROOT / 'packaging/fetch-steamos-runtime.sh'


class ShortcutsTests(unittest.TestCase):
    def test_standard_double_terminated_vdf_is_readable(self):
        # Literal Valve-style root and outer object terminators, independent
        # of the writer under test.
        fixture = b'\x00shortcuts\x00\x00' + b'0\x00\x01AppName\x00Other Game\x00\x08\x08\x08'
        self.assertEqual(shortcuts.read_vdf(fixture), [{'AppName': 'Other Game'}])
        self.assertTrue(shortcuts.write_vdf([{'AppName': 'Other Game'}]).endswith(b'\x08\x08\x08'))

    def test_update_preserves_unrelated_entry_and_backups_original(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'shortcuts.vdf'
            original = shortcuts.write_vdf([{'AppName': 'Other Game', 'Exe': 'other', 'tags': {'0': 'keep'}}])
            path.write_bytes(original)
            shortcuts.update(path, Path(directory) / 'skjalf.sh', Path(directory))
            updated = shortcuts.read_vdf(path.read_bytes())
            self.assertEqual([item['AppName'] for item in updated], ['Other Game', 'Skjalf TV'])
            self.assertEqual(updated[0]['tags'], {'0': 'keep'})
            self.assertEqual(path.with_name('shortcuts.vdf.bak').read_bytes(), original)
            updated[1]['tags'] = {'0': 'user tag'}
            path.write_bytes(shortcuts.write_vdf(updated))
            shortcuts.update(path, Path(directory) / 'skjalf.sh', Path(directory))
            reinstalled = shortcuts.read_vdf(path.read_bytes())
            self.assertEqual(len(reinstalled), 2)
            self.assertEqual(reinstalled[1]['tags'], {'0': 'user tag'})
            self.assertNotEqual(reinstalled[1]['appid'], 0)

    def test_update_replaces_the_old_steamtv_entry_and_keeps_similar_names(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'shortcuts.vdf'
            path.write_bytes(shortcuts.write_vdf([
                {'AppName': 'SteamTV', 'Exe': 'old'},
                {'AppName': 'SteamTV Test', 'Exe': 'private'},
                {'AppName': 'Other Game', 'Exe': 'other'},
            ]))
            shortcuts.update(path, Path(directory) / 'skjalf.sh', Path(directory))
            names = [item['AppName'] for item in shortcuts.read_vdf(path.read_bytes())]
            self.assertEqual(names, ['SteamTV Test', 'Other Game', 'Skjalf TV'])

    def test_update_carries_user_settings_over_from_the_old_steamtv_entry(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'shortcuts.vdf'
            path.write_bytes(shortcuts.write_vdf([{
                'appid': 1, 'AppName': 'SteamTV', 'Exe': 'old', 'StartDir': 'old', 'icon': 'old.png',
                'LaunchOptions': '--fast', 'AllowOverlay': 0, 'openvr': 1, 'LastPlayTime': 77,
                'tags': {'0': 'TV'},
            }]))
            shortcuts.update(path, Path(directory) / 'skjalf.sh', Path(directory))
            [entry] = shortcuts.read_vdf(path.read_bytes())
            self.assertEqual(entry['AppName'], 'Skjalf TV')
            self.assertEqual(entry['Exe'], f'"{Path(directory) / "skjalf.sh"}"')
            self.assertEqual(entry['StartDir'], f'"{directory}"')
            self.assertEqual(entry['icon'], str(Path(directory) / 'skjalf.png'))
            self.assertNotEqual(entry['appid'], 1)
            self.assertEqual(entry['LaunchOptions'], '--fast')
            self.assertEqual((entry['AllowOverlay'], entry['openvr'], entry['LastPlayTime']), (0, 1, 77))
            self.assertEqual(entry['tags'], {'0': 'TV'})

    def test_update_sets_the_icon_and_writes_exactly_the_four_grid_files(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            path = base / 'config' / 'shortcuts.vdf'
            path.parent.mkdir()
            grid = path.parent / 'grid'
            grid.mkdir()
            (grid / '123p.png').write_bytes(b'user art')
            shortcuts.update(path, base / 'skjalf.sh', base)
            [entry] = shortcuts.read_vdf(path.read_bytes())
            self.assertEqual(entry['icon'], str(base / 'skjalf.png'))
            appid = entry['appid'] & 0xFFFFFFFF
            art = ROOT / 'packaging/icons'
            expected = {
                f'{appid}p.png': 'capsule.png', f'{appid}.png': 'header.png',
                f'{appid}_hero.png': 'hero.png', f'{appid}_logo.png': 'logo.png',
            }
            self.assertEqual(sorted(p.name for p in grid.iterdir()), sorted([*expected, '123p.png']))
            for name, source in expected.items():
                self.assertEqual((grid / name).read_bytes(), (art / source).read_bytes())
            self.assertEqual((grid / '123p.png').read_bytes(), b'user art')

    def test_update_creates_a_missing_grid_directory_and_leaves_old_appid_art(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            path = base / 'shortcuts.vdf'
            path.write_bytes(shortcuts.write_vdf([{'appid': -5, 'AppName': 'SteamTV', 'Exe': 'old'}]))
            old_art = base / 'grid'
            old_art.mkdir()
            (old_art / f'{-5 & 0xFFFFFFFF}p.png').write_bytes(b'old')
            shortcuts.update(path, base / 'skjalf.sh', base)
            self.assertEqual((old_art / f'{-5 & 0xFFFFFFFF}p.png').read_bytes(), b'old')
            self.assertEqual(len(list(old_art.iterdir())), 5)
            fresh = Path(directory) / 'other'
            fresh.mkdir()
            shortcuts.update(fresh / 'shortcuts.vdf', base / 'skjalf.sh', base)
            self.assertEqual(len(list((fresh / 'grid').iterdir())), 4)

    def test_corrupt_shortcuts_are_left_untouched(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'shortcuts.vdf'
            path.write_bytes(b'bad fixture')
            with self.assertRaises(ValueError):
                shortcuts.update(path, Path(directory) / 'skjalf.sh', Path(directory))
            self.assertEqual(path.read_bytes(), b'bad fixture')
            self.assertFalse(path.with_name('shortcuts.vdf.bak').exists())


class RuntimeFetchTests(unittest.TestCase):
    def test_lock_file_lists_eight_pinned_packages(self):
        entries = [line.split() for line in LOCK.read_text().splitlines() if line.strip() and not line.startswith('#')]
        self.assertEqual(len(entries), 8)
        for sha, url, role in entries:
            self.assertRegex(sha, r'^[0-9a-f]{64}$')
            self.assertTrue(url.startswith('https://steamdeck-packages.steamos.cloud/'), url)
            self.assertIn(role, ('runtime', 'tool'))
        self.assertEqual([role for _, _, role in entries].count('tool'), 1)

    def test_hash_mismatch_fails_and_leaves_no_lib_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            cache = base / 'cache'
            cache.mkdir()
            payload = base / 'payload/usr/lib'
            payload.mkdir(parents=True)
            (payload / 'libfake.so.1').write_bytes(b'not a real library')
            package = cache / 'fake-1.0-1-x86_64.pkg.tar.zst'
            subprocess.run(['tar', '--zstd', '-cf', str(package), '-C', str(base / 'payload'), 'usr'], check=True)
            lock = base / 'test.lock'
            lock.write_text(f"{'0' * 64} https://steamdeck-packages.steamos.cloud/{package.name} runtime\n")
            out = base / 'out'
            env = dict(os.environ, SKJALF_PACKAGE_CACHE=str(cache), SKJALF_RUNTIME_LOCK=str(lock))
            result = subprocess.run(['sh', str(FETCH), str(out)], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(package.name, result.stderr)
            self.assertIn('mismatch', result.stderr)
            self.assertFalse((out / 'lib').exists())

    def test_failed_dependency_inspection_fails_and_leaves_no_lib_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            cache = base / 'cache'
            cache.mkdir()
            lines = []
            for name, relative, content, mode in (
                ('fakempv-1.0-1-x86_64.pkg.tar.zst', 'usr/lib/libmpv.so.2', b'not elf', 0o644),
                ('fakepatchelf-1.0-1-x86_64.pkg.tar.zst', 'usr/bin/patchelf', b'#!/bin/sh\n[ "$1" = --print-needed ] && exit 1\nexit 0\n', 0o755),
            ):
                payload = base / name
                target = payload / relative
                target.parent.mkdir(parents=True)
                target.write_bytes(content)
                target.chmod(mode)
                (payload / '.PKGINFO').write_text(f'pkgname = {name.split("-")[0]}\npkgver = 1.0-1\n')
                package = cache / name
                subprocess.run(['tar', '--zstd', '-cf', str(package), '-C', str(payload), '.PKGINFO', relative.split('/')[0]], check=True)
                role = 'tool' if 'patchelf' in name else 'runtime'
                lines.append(f'{hashlib.sha256(package.read_bytes()).hexdigest()} https://steamdeck-packages.steamos.cloud/{name} {role}')
            lock = base / 'test.lock'
            lock.write_text('\n'.join(lines) + '\n')
            out = base / 'out'
            env = dict(os.environ, SKJALF_PACKAGE_CACHE=str(cache), SKJALF_RUNTIME_LOCK=str(lock))
            result = subprocess.run(['sh', str(FETCH), str(out)], env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((out / 'lib').exists())

    @unittest.skipUnless(os.environ.get('SKJALF_TEST_NETWORK') == '1', 'set SKJALF_TEST_NETWORK=1 to download the runtime')
    def test_real_runtime_is_complete_and_patched(self):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / 'runtime'
            result = subprocess.run(['sh', str(FETCH), str(out)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            for name in ('libmpv.so.2', 'libmujs.so', 'libXpresent.so.1', 'libcdio.so.19', 'libsixel.so.1', 'libuchardet.so.0'):
                self.assertTrue((out / 'lib' / name).exists(), name)
            dynamic = subprocess.run(['readelf', '-d', str(out / 'lib/libmpv.so.2')], capture_output=True, text=True, check=True).stdout
            needed = re.findall(r'\(NEEDED\)\s+Shared library: \[(.*?)\]', dynamic)
            self.assertIn('libmujs.so', needed)
            self.assertNotIn('/usr/lib/libmujs.so', needed)
            manifest = (out / 'runtime.txt').read_text()
            self.assertIn('mpv ', manifest)
            self.assertIn('patchelf ', manifest)


@unittest.skipUnless(BINARY.is_file() and VENDOR.is_dir(), 'built app and vendored libmpv required')
class BundleTests(unittest.TestCase):
    def test_dependency_stage_reports_host_libraries_and_missing_vendor(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            output = base / 'good'
            result = subprocess.run(['python3', str(ROOT / 'packaging/check-deps.py'), str(BINARY), str(VENDOR), str(output)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            report = (output / 'dependencies.txt').read_text()
            self.assertIn('libmpv.so.2', report)
            self.assertIn('libXpresent.so.1', report)
            self.assertIn('Host libraries required', report)
            self.assertTrue((output / 'lib/libmpv.so.2').exists())
            bad = base / 'empty-vendor'
            bad.mkdir()
            result = subprocess.run(['python3', str(ROOT / 'packaging/check-deps.py'), str(BINARY), str(bad), str(base / 'bad')], capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('required bundled library missing', result.stderr)

    def test_dependency_stage_copies_every_library_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / 'stage'
            result = subprocess.run(['python3', str(ROOT / 'packaging/check-deps.py'), str(BINARY), str(VENDOR), str(output)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            expected = sorted(p.name for p in VENDOR.glob('*.so*'))
            self.assertIn('libmpv.so.2', expected)
            self.assertIn('libXpresent.so.1', expected)
            self.assertEqual(sorted(p.name for p in (output / 'lib').iterdir()), expected)
            for source in VENDOR.glob('*.so*'):
                self.assertEqual((output / 'lib' / source.name).is_symlink(), source.is_symlink(), source.name)
            for name in expected:
                self.assertIn(f'  {name}\n', result.stdout)

    def test_installer_writes_only_fixture_home(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory) / 'space & name'
            config = home / '.local/share/Steam/userdata/42/config'
            config.mkdir(parents=True)
            anonymous = home / '.local/share/Steam/userdata/0/config'
            anonymous.mkdir(parents=True)
            path = config / 'shortcuts.vdf'
            path.write_bytes(shortcuts.write_vdf([{'AppName': 'Other Game', 'Exe': 'other'}]))
            env = os.environ.copy()
            env['SKJALF_INSTALL_HOME'] = str(home)
            env['SKJALF_MPV_LIB_DIR'] = str(VENDOR)
            result = subprocess.run(['sh', str(ROOT / 'packaging/install-steamos.sh'), str(BINARY)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue((home / '.local/share/skjalf/lib/libmpv.so.2').is_file())
            self.assertTrue((home / '.local/share/skjalf/lib/libXpresent.so.1').is_file())
            self.assertTrue((home / '.local/share/skjalf/dependencies.txt').is_file())
            self.assertEqual((home / '.local/share/skjalf/skjalf.png').read_bytes(), (ROOT / 'packaging/icons/skjalf-256.png').read_bytes())
            for size in (32, 48, 64, 128, 256, 512):
                self.assertEqual(
                    (home / f'.local/share/icons/hicolor/{size}x{size}/apps/skjalf.png').read_bytes(),
                    (ROOT / f'packaging/icons/skjalf-{size}.png').read_bytes(),
                )
            self.assertTrue((home / '.local/share/icons/hicolor/scalable/apps/skjalf.svg').is_file())
            [entry] = [e for e in shortcuts.read_vdf(path.read_bytes()) if e['AppName'] == 'Skjalf TV']
            self.assertEqual(entry['icon'], f'{home}/.local/share/skjalf/skjalf.png')
            self.assertEqual(len(list((config / 'grid').iterdir())), 4)
            desktop = (home / '.local/share/applications/skjalf.desktop').read_text()
            self.assertIn(f'Exec="{home}/.local/share/skjalf/skjalf.sh"', desktop)
            self.assertEqual([item['AppName'] for item in shortcuts.read_vdf(path.read_bytes())], ['Other Game', 'Skjalf TV'])
            self.assertEqual(list(anonymous.iterdir()), [])


if __name__ == '__main__':
    unittest.main()
