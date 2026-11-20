import hashlib
import importlib.util
import os
import re
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
VENDOR = Path(os.environ.get('SKJALF_TEST_MPV_LIB_DIR', ROOT / '.deps/mpv/usr/lib/x86_64-linux-gnu'))
BINARY = Path(os.environ.get('SKJALF_TEST_BINARY', ROOT / 'target/debug/skjalf'))

spec = importlib.util.spec_from_file_location('shortcuts', ROOT / 'packaging/update-shortcuts.py')
shortcuts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcuts)

FAKE_PGREP = """#!/bin/sh
for name; do :; done
case "$name" in
    steam|steamwebhelper) [ -f "$FAKE/steam-running" ] ;;
    gamescope) [ -f "$FAKE/gamescope" ] ;;
    *) false ;;
esac
"""
FAKE_SYSTEMCTL = """#!/bin/sh
[ -f "$FAKE/gamescope-session" ]
"""
FAKE_STEAM = """#!/bin/sh
echo "$@" >> "$FAKE/calls"
[ -f "$FAKE/steam-stays" ] || rm -f "$FAKE/steam-running"
"""
ASSETS = ['skjalf-tv-x86_64.tar.gz', 'install.sh', 'uninstall.sh', 'SHA256SUMS']


def make_release(binary, out):
    return subprocess.run(['sh', str(ROOT / 'packaging/make-release.sh'), str(binary), str(out)], capture_output=True, text=True)


def fake_environment(base):
    fake = base / 'fake'
    bin_dir = base / 'fakebin'
    fake.mkdir()
    bin_dir.mkdir()
    for name, body in (('pgrep', FAKE_PGREP), ('systemctl', FAKE_SYSTEMCTL), ('steam', FAKE_STEAM)):
        script = bin_dir / name
        script.write_text(body)
        script.chmod(0o755)
    env = os.environ.copy()
    env.pop('XDG_CONFIG_HOME', None)
    env.update(FAKE=str(fake), PATH=f'{bin_dir}:{env["PATH"]}')
    return fake, env


WORKSPACE_VERSION = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M).group(1)

class MakeReleaseTests(unittest.TestCase):
    def test_assets_layout_and_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            binary = base / 'skjalf'
            binary.write_bytes(b'#!/bin/sh\n')
            binary.chmod(0o755)
            result = make_release(binary, base / 'out')
            self.assertEqual(result.returncode, 0, result.stderr)
            out = base / 'out'
            self.assertEqual(sorted(p.name for p in out.iterdir()), sorted(ASSETS))
            with tarfile.open(out / 'skjalf-tv-x86_64.tar.gz') as tar:
                names = tar.getnames()
                self.assertTrue(all(name.startswith('skjalf-tv') for name in names))
                for required in (
                    'skjalf-tv/skjalf', 'skjalf-tv/README.md', 'skjalf-tv/LICENSE', 'skjalf-tv/VERSION',
                    'skjalf-tv/packaging/install-steamos.sh', 'skjalf-tv/packaging/uninstall-steamos.sh',
                    'skjalf-tv/packaging/update-shortcuts.py', 'skjalf-tv/packaging/steamos-runtime.lock',
                    'skjalf-tv/packaging/skjalf.desktop', 'skjalf-tv/packaging/icons/skjalf-256.png',
                ):
                    self.assertIn(required, names)
                self.assertFalse([n for n in names if '/tests' in n or '__pycache__' in n])
                self.assertEqual(tar.extractfile('skjalf-tv/VERSION').read().decode().strip(), WORKSPACE_VERSION)
                self.assertTrue(tar.getmember('skjalf-tv/skjalf').mode & 0o111)
            self.assertEqual((out / 'install.sh').read_bytes(), (ROOT / 'packaging/bootstrap-install.sh').read_bytes())
            checked = subprocess.run(['sha256sum', '-c', 'SHA256SUMS'], cwd=out, capture_output=True, text=True)
            self.assertEqual(checked.returncode, 0, checked.stderr)
            listed = {line.split()[1] for line in (out / 'SHA256SUMS').read_text().splitlines()}
            self.assertTrue({'skjalf-tv-x86_64.tar.gz', 'install.sh'} <= listed)

    def test_release_is_reproducible(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            binary = base / 'skjalf'
            binary.write_bytes(b'bin')
            binary.chmod(0o755)
            digests = []
            for name in ('a', 'b'):
                self.assertEqual(make_release(binary, base / name).returncode, 0)
                digests.append(hashlib.sha256((base / name / 'skjalf-tv-x86_64.tar.gz').read_bytes()).hexdigest())
            self.assertEqual(digests[0], digests[1])


@unittest.skipUnless(BINARY.is_file() and VENDOR.is_dir(), 'built app and vendored libmpv required')
class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.release = self.base / 'release'
        self.assertEqual(make_release(BINARY, self.release).returncode, 0)
        self.home = self.base / 'home'
        self.home.mkdir()
        self.fake, self.env = fake_environment(self.base)
        self.env.update(
            SKJALF_RELEASE_URL=f'file://{self.release}',
            SKJALF_INSTALL_HOME=str(self.home),
            SKJALF_MPV_LIB_DIR=str(VENDOR),
            SKJALF_STEAM_WAIT='1',
        )

    def run_bootstrap(self):
        return subprocess.run(['sh', str(self.release / 'install.sh')], env=self.env, capture_output=True, text=True)

    def installed(self):
        return (self.home / '.local/share/skjalf').exists()

    def test_installs_from_the_release_assets(self):
        result = self.run_bootstrap()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.home / '.local/share/skjalf/skjalf').is_file())
        self.assertTrue((self.home / '.local/share/skjalf/lib/libmpv.so.2').is_file())
        self.assertTrue((self.home / '.local/share/applications/skjalf.desktop').is_file())
        self.assertIn('Non-Steam', result.stdout)
        self.assertFalse((self.fake / 'calls').exists())

    def test_corrupted_tarball_stops_and_installs_nothing(self):
        with open(self.release / 'skjalf-tv-x86_64.tar.gz', 'ab') as stream:
            stream.write(b'x')
        result = self.run_bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('checksum', result.stderr)
        self.assertFalse(self.installed())

    def test_game_mode_stops_with_a_desktop_mode_message(self):
        (self.fake / 'steam-running').touch()
        (self.fake / 'gamescope-session').touch()
        result = self.run_bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Desktop Mode', result.stderr)
        self.assertFalse(self.installed())
        self.assertFalse((self.fake / 'calls').exists())

    def test_desktop_mode_closes_a_running_steam_first(self):
        (self.fake / 'steam-running').touch()
        result = self.run_bootstrap()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.fake / 'calls').read_text().strip(), '-shutdown')
        self.assertIn('Steam is running and will close', result.stdout)
        self.assertTrue(self.installed())

    def test_steam_that_does_not_close_stops_the_install(self):
        (self.fake / 'steam-running').touch()
        (self.fake / 'steam-stays').touch()
        result = self.run_bootstrap()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('did not close', result.stderr)
        self.assertFalse(self.installed())


class UninstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.home = self.base / 'home'
        self.dest = self.home / '.local/share/skjalf'
        (self.dest / 'lib').mkdir(parents=True)
        (self.dest / 'skjalf').write_bytes(b'bin')
        icons = self.home / '.local/share/icons/hicolor'
        self.theme_files = [icons / f'{s}x{s}/apps/skjalf.png' for s in (32, 48, 64, 128, 256, 512)]
        self.theme_files.append(icons / 'scalable/apps/skjalf.svg')
        self.other_icon = icons / '48x48/apps/other.png'
        for path in [*self.theme_files, self.other_icon]:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'icon')
        applications = self.home / '.local/share/applications'
        applications.mkdir(parents=True)
        self.desktop = applications / 'skjalf.desktop'
        self.desktop.write_text('[Desktop Entry]\n')
        self.other_desktop = applications / 'other.desktop'
        self.other_desktop.write_text('[Desktop Entry]\n')
        self.config = self.home / '.config/skjalf'
        self.config.mkdir(parents=True)
        (self.config / 'settings.toml').write_text('volume = 50\n')

        userdata = self.home / '.local/share/Steam/userdata'
        self.vdf = userdata / '42/config/shortcuts.vdf'
        self.anonymous_vdf = userdata / '0/config/shortcuts.vdf'
        for path in (self.vdf, self.anonymous_vdf):
            path.parent.mkdir(parents=True)
            path.write_bytes(shortcuts.write_vdf([
                {'appid': 7, 'AppName': 'Other Game', 'Exe': 'other'},
                {'appid': 8, 'AppName': 'Skjalf TV Test', 'Exe': 'test'},
            ]))
            shortcuts.update(path, self.dest / 'skjalf.sh', self.dest)
        self.grid = self.vdf.parent / 'grid'
        (self.grid / 'keep.png').write_bytes(b'keep')
        (self.grid / '7p.png').write_bytes(b'other game art')
        self.anonymous_before = self.anonymous_vdf.read_bytes()
        self.fake, self.env = fake_environment(self.base)
        self.env.update(SKJALF_INSTALL_HOME=str(self.home))

    def run_uninstall(self, *args):
        return subprocess.run(['sh', str(ROOT / 'packaging/uninstall-steamos.sh'), *args], env=self.env, capture_output=True, text=True)

    def names(self, path):
        return [item['AppName'] for item in shortcuts.read_vdf(path.read_bytes())]

    def test_removes_only_the_app_files_entry_and_its_artwork(self):
        self.assertEqual(len(list(self.grid.iterdir())), 6)
        result = self.run_uninstall()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.dest.exists())
        self.assertFalse(self.desktop.exists())
        self.assertTrue(self.other_desktop.exists())
        self.assertFalse(any(path.exists() for path in self.theme_files))
        self.assertTrue(self.other_icon.exists())
        self.assertEqual(self.names(self.vdf), ['Other Game', 'Skjalf TV Test'])
        self.assertTrue(self.vdf.with_name('shortcuts.vdf.bak').exists())
        self.assertEqual(sorted(p.name for p in self.grid.iterdir()), ['7p.png', 'keep.png'])
        self.assertEqual(self.anonymous_vdf.read_bytes(), self.anonymous_before)
        self.assertTrue((self.config / 'settings.toml').exists())
        self.assertIn('--purge', result.stdout)

    def test_purge_removes_the_settings_directory(self):
        result = self.run_uninstall('--purge')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.config.exists())
        self.assertIn('credentials', result.stdout)

    def test_installed_copy_removes_the_entry_too(self):
        shortcuts_copy = self.dest / 'update-shortcuts.py'
        shortcuts_copy.write_bytes((ROOT / 'packaging/update-shortcuts.py').read_bytes())
        standalone = self.base / 'uninstall.sh'
        standalone.write_bytes((ROOT / 'packaging/uninstall-steamos.sh').read_bytes())
        result = subprocess.run(['sh', str(standalone)], env=self.env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.names(self.vdf), ['Other Game', 'Skjalf TV Test'])

    def test_game_mode_stops_before_removing_anything(self):
        (self.fake / 'steam-running').touch()
        (self.fake / 'gamescope-session').touch()
        result = self.run_uninstall()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Desktop Mode', result.stderr)
        self.assertTrue(self.dest.exists())



def release_notes(version, changelog=None):
    args = ['sh', str(ROOT / 'packaging/release-notes.sh'), version]
    if changelog:
        args.append(str(changelog))
    return subprocess.run(args, capture_output=True, text=True)


class ReleaseNotesTests(unittest.TestCase):
    CHANGELOG = """# Changelog

## 0.2.0 - 2026-01-02

- Added the new thing.

## 0.1.0 - 2026-01-01

- First release.
"""

    def test_notes_hold_only_the_versions_changes_and_the_install_line(self):
        with tempfile.TemporaryDirectory() as directory:
            changelog = Path(directory) / 'CHANGELOG.md'
            changelog.write_text(self.CHANGELOG)
            result = release_notes('0.2.0', changelog)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('- Added the new thing.', result.stdout)
            self.assertNotIn('First release', result.stdout)
            self.assertNotIn('## ', result.stdout)
            self.assertIn('releases/latest/download/install.sh | sh', result.stdout)
            self.assertIn('SHA256SUMS', result.stdout)

    def test_a_version_without_changes_stops_the_release(self):
        with tempfile.TemporaryDirectory() as directory:
            changelog = Path(directory) / 'CHANGELOG.md'
            changelog.write_text(self.CHANGELOG)
            result = release_notes('0.3.0', changelog)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('0.3.0', result.stderr)
            self.assertEqual(result.stdout, '')

    def test_the_workspace_version_has_a_changelog_entry(self):
        result = release_notes(WORKSPACE_VERSION)
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
