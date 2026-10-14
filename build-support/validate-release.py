#!/usr/bin/env python3
"""Validate a release tag, its main CI run, and the tested portal artifact."""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def output(command):
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def validate_tag(tag, version, actual_commit, expected_commit):
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?', tag):
        raise ValueError('Use a numeric release tag without a v prefix')
    if tag != version:
        raise ValueError(f'Tag {tag} differs from workspace version {version}')
    if actual_commit != expected_commit:
        raise ValueError('Release tag moved after the event')


def require_ci(commit):
    runs = json.loads(output([
        'gh', 'run', 'list', '--workflow', 'ci.yml', '--commit', commit,
        '--event', 'push', '--branch', 'main', '--limit', '1',
        '--json', 'databaseId,headSha',
    ]))
    if not runs or runs[0]['headSha'] != commit:
        raise ValueError('No main push CI run exists for this exact commit. Merge to main and run CI before releasing.')
    run_id = runs[0]['databaseId']
    # Wait for pending CI and fail for failed/cancelled runs, including completed ones.
    subprocess.run(['gh', 'run', 'watch', str(run_id),
                    '--exit-status', '--interval', '15'], cwd=ROOT, check=True)
    run = json.loads(output(['gh', 'api', f'repos/{{owner}}/{{repo}}/actions/runs/{run_id}']))
    if (run['head_sha'] != commit or run['head_branch'] != 'main' or run['event'] != 'push'
            or run['status'] != 'completed' or run['conclusion'] != 'success'):
        raise ValueError('The selected main CI run is no longer a successful run for this commit')
    return run_id, run['run_attempt']


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate_artifacts(directory, tag, commit):
    base = f'wifi-setupd-web-{tag}'
    archive = directory / f'{base}.tar.gz'
    metadata = directory / f'{base}.provenance.json'
    checksums = directory / f'{base}.sha256'
    expected = {archive, metadata, checksums}
    if set(directory.iterdir()) != expected or any(p.is_symlink() or not p.is_file() for p in expected):
        raise ValueError('Expected exactly the portal archive, provenance and checksum files for the release tag')
    expected_checksums = ''.join(f'{sha256(p)}  {p.name}\n' for p in (archive, metadata))
    if checksums.read_text() != expected_checksums:
        raise ValueError('Release artifact checksums do not match')
    provenance = json.loads(metadata.read_text())
    if (provenance.get('version') != tag or provenance.get('source_commit') != commit
            or provenance.get('source_dirty') is not False
            or provenance.get('cargo_lock_sha256') != sha256(ROOT / 'Cargo.lock')
            or provenance.get('archive_sha256') != sha256(archive)):
        raise ValueError('Artifact provenance does not match the clean release commit, version and Cargo.lock')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--require-ci', action='store_true')
    parser.add_argument('--artifact-dir', type=Path)
    args = parser.parse_args()
    try:
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        validate_tag(args.tag, version, output(['git', 'rev-parse', 'HEAD']), args.commit)
        if args.require_ci:
            run_id, attempt = require_ci(args.commit)
            if os.environ.get('GITHUB_OUTPUT'):
                with open(os.environ['GITHUB_OUTPUT'], 'a') as destination:
                    destination.write(f'ci_run_id={run_id}\nci_run_attempt={attempt}\n')
        if args.artifact_dir:
            validate_artifacts(args.artifact_dir, args.tag, args.commit)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'release validation: {error}', file=sys.stderr)
        return 1
    print('Release validation passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
