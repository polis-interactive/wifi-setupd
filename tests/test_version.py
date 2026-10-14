"""Version changes keep independent lockfiles in sync and preserve edits on failure."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'build-support/version.py'
SPEC = importlib.util.spec_from_file_location('version', SCRIPT)
version = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(version)


class VersionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.manifest = self.root / 'Cargo.toml'
        self.manifest.write_text('[workspace.package]\nversion = "1.2.3" # keep this\n'
                                 'edition = "2024"\n\n[workspace.dependencies]\n'
                                 'example = { version = "1.2.3" }\n')
        for name in version.MANIFESTS:
            lock = self.root / Path(name).with_name('Cargo.lock')
            lock.parent.mkdir(parents=True, exist_ok=True)
            lock.write_text(f'Existing local edits in {name}\n')

    def test_major_minor_patch_reset_lower_components(self):
        self.assertEqual([version.next_version('1.2.3', n) for n in ('1', '2', '3')],
                         ['2.0.0', '1.3.0', '1.2.4'])
        self.assertEqual(version.next_version('0.1.2', '2'), '0.2.0')

    def test_blank_and_eof_cancel_without_changes(self):
        original = self.manifest.read_bytes()
        for answer in ('', EOFError()):
            with patch.object(version, 'ROOT', self.root), patch('sys.argv', ['version.py']), \
                    patch('builtins.input', side_effect=[answer]), \
                    patch.object(version, 'sync_lockfiles') as sync:
                self.assertEqual(version.main(), 0)
                sync.assert_not_called()
                self.assertEqual(self.manifest.read_bytes(), original)

    def test_invalid_choice_retries_then_bumps_only_workspace_version(self):
        with patch.object(version, 'ROOT', self.root), patch('sys.argv', ['version.py']), \
                patch('builtins.input', side_effect=['4', '3']), \
                patch.object(version, 'sync_lockfiles') as sync:
            self.assertEqual(version.main(), 0)
            sync.assert_called_once_with(self.root)
        data = tomllib.loads(self.manifest.read_text())
        self.assertEqual(data['workspace']['package']['version'], '1.2.4')
        self.assertEqual(data['workspace']['dependencies']['example']['version'], '1.2.3')
        self.assertIn('# keep this', self.manifest.read_text())

    def test_failed_or_interrupted_sync_restores_all_original_files(self):
        original = {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        for failure in (subprocess.CalledProcessError(1, ['cargo']), KeyboardInterrupt()):
            def fail(_):
                for name in version.MANIFESTS:
                    (self.root / Path(name).with_name('Cargo.lock')).write_text('partial update')
                raise failure
            with patch.object(version, 'sync_lockfiles', side_effect=fail):
                with self.assertRaises(type(failure)):
                    version.bump_version(self.root, '2.0.0')
            self.assertEqual(original, {p: p.read_bytes() for p in original})

    def test_sync_refreshes_every_workspace_conservatively(self):
        with patch.object(version.subprocess, 'run') as run:
            version.sync_lockfiles(self.root)
        self.assertEqual(len(run.call_args_list), 4)
        for call, manifest in zip(run.call_args_list, version.MANIFESTS):
            self.assertEqual(call.args[0], ['cargo', 'update', '--workspace', '--offline',
                                           '--manifest-path', manifest])
            self.assertTrue(call.kwargs['check'])


if __name__ == '__main__':
    unittest.main()
