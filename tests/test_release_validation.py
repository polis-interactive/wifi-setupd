"""Release gating must reject untested commits and propagate CI failure."""
import importlib.util
import json
from pathlib import Path
import subprocess
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

    def test_successful_ci_uses_the_exact_commit_and_enforces_exit_status(self):
        runs = [{'databaseId': 42, 'headSha': 'release-commit'}]
        with patch.object(validation, 'output', return_value=json.dumps(runs)) as query, \
                patch.object(validation.subprocess, 'run') as watch:
            validation.require_ci('release-commit')
            args = query.call_args.args[0]
            self.assertEqual(args[args.index('--commit') + 1], 'release-commit')
            self.assertEqual(args[args.index('--event') + 1], 'push')
            self.assertIn('--exit-status', watch.call_args.args[0])
            self.assertTrue(watch.call_args.kwargs['check'])


if __name__ == '__main__':
    unittest.main()
