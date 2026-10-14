"""Release gating must reject untested commits and mismatched build artifacts."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / 'build-support/validate-release.py'
SPEC = importlib.util.spec_from_file_location('validate_release', SCRIPT)
validation = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(validation)


class ReleaseValidationTests(unittest.TestCase):
    def test_tag_and_commit_must_match(self):
        validation.validate_tag('0.1.1', '0.1.1', 'commit', 'commit')
        validation.validate_tag('0.1.1-rc.1', '0.1.1-rc.1', 'commit', 'commit')
        for tag, version, actual in (
            ('v0.1.1', '0.1.1', 'commit'),
            ('0.1.0', '0.1.1', 'commit'),
            ('0.1.1', '0.1.1', 'moved-commit'),
        ):
            with self.subTest(tag=tag, actual=actual), self.assertRaises(ValueError):
                validation.validate_tag(tag, version, actual, 'commit')

    def test_missing_or_different_commit_cannot_satisfy_gate(self):
        for runs in ([], [{'databaseId': 1, 'headSha': 'other-commit'}]):
            with patch.object(validation, 'output', return_value=json.dumps(runs)), \
                    patch.object(validation.subprocess, 'run') as watch:
                with self.assertRaises(ValueError):
                    validation.require_ci('release-commit')
                watch.assert_not_called()

    def test_failed_or_cancelled_ci_stops_release(self):
        runs = [{'databaseId': 42, 'headSha': 'release-commit'}]
        failure = subprocess.CalledProcessError(1, ['gh', 'run', 'watch'])
        with patch.object(validation, 'output', return_value=json.dumps(runs)), \
                patch.object(validation.subprocess, 'run', side_effect=failure):
            with self.assertRaises(subprocess.CalledProcessError):
                validation.require_ci('release-commit')

    def test_successful_main_ci_returns_run_and_attempt_for_exact_commit(self):
        runs = [{'databaseId': 42, 'headSha': 'release-commit'}]
        run = dict(head_sha='release-commit', head_branch='main', event='push',
                   status='completed', conclusion='success', run_attempt=2)
        with patch.object(validation, 'output', side_effect=[json.dumps(runs), json.dumps(run)]) as query, \
                patch.object(validation.subprocess, 'run') as watch:
            self.assertEqual(validation.require_ci('release-commit'), (42, 2))
            args = query.call_args_list[0].args[0]
            self.assertEqual(args[args.index('--commit') + 1], 'release-commit')
            self.assertEqual(args[args.index('--event') + 1], 'push')
            self.assertEqual(args[args.index('--branch') + 1], 'main')
            self.assertIn('--exit-status', watch.call_args.args[0])
            self.assertTrue(watch.call_args.kwargs['check'])

    def test_pr_branch_or_rerunning_ci_cannot_supply_release(self):
        runs = [{'databaseId': 42, 'headSha': 'release-commit'}]
        good_run = dict(head_sha='release-commit', head_branch='main', event='push',
                        status='completed', conclusion='success', run_attempt=2)
        for field, value in (('head_sha', 'other'), ('head_branch', 'feature'),
                             ('event', 'pull_request'), ('status', 'in_progress'),
                             ('conclusion', 'failure')):
            with self.subTest(field=field), \
                    patch.object(validation, 'output', side_effect=[
                        json.dumps(runs), json.dumps({**good_run, field: value}),
                    ]), patch.object(validation.subprocess, 'run'):
                with self.assertRaises(ValueError):
                    validation.require_ci('release-commit')


class ArtifactValidationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.archive = self.directory / 'wifi-setupd-web-0.1.1.tar.gz'
        self.metadata = self.directory / 'wifi-setupd-web-0.1.1.provenance.json'
        self.checksums = self.directory / 'wifi-setupd-web-0.1.1.sha256'
        self.archive.write_bytes(b'tested archive fixture')
        self.provenance = dict(version='0.1.1', source_commit='commit', source_dirty=False,
                               cargo_lock_sha256=validation.sha256(validation.ROOT / 'Cargo.lock'),
                               archive_sha256=validation.sha256(self.archive))
        self.write_metadata()

    def write_metadata(self):
        self.metadata.write_text(json.dumps(self.provenance))
        self.checksums.write_text(''.join(
            f'{validation.sha256(path)}  {path.name}\n' for path in (self.archive, self.metadata)))

    def validate(self):
        validation.validate_artifacts(self.directory, '0.1.1', 'commit')

    def test_accepts_matching_artifact_without_changing_bytes(self):
        before = {p.name: p.read_bytes() for p in self.directory.iterdir()}
        self.validate()
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.directory.iterdir()})

    def test_provenance_must_match_even_with_valid_checksums(self):
        for field, value in (('version', '0.1.2'), ('source_commit', 'other'),
                             ('source_dirty', True), ('source_dirty', None),
                             ('cargo_lock_sha256', 'wrong'), ('archive_sha256', 'wrong')):
            with self.subTest(field=field), patch.dict(self.provenance, {field: value}):
                self.write_metadata()
                with self.assertRaises(ValueError):
                    self.validate()

    def test_rejects_corruption_and_missing_checksums(self):
        self.archive.write_bytes(b'changed')
        with self.assertRaises(ValueError):
            self.validate()
        self.checksums.unlink()
        with self.assertRaises(ValueError):
            self.validate()

    def test_rejects_unexpected_assets(self):
        (self.directory / 'unrelated-secret').write_text('must not publish')
        with self.assertRaises(ValueError):
            self.validate()


if __name__ == '__main__':
    unittest.main()
