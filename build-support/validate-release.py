#!/usr/bin/env python3
"""Validate a release tag and require CI on its commit or the PR that merged it."""
import argparse
import json
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


def latest_pr_run(commit):
    runs = json.loads(output([
        'gh', 'run', 'list', '--workflow', 'ci.yml', '--commit', commit,
        '--event', 'pull_request', '--limit', '1', '--json', 'databaseId,headSha',
    ]))
    return runs[0] if runs and runs[0]['headSha'] == commit else None


def require_ci(commit):
    run = latest_pr_run(commit)
    if run is None:
        # Merge/squash/rebase can give the release a different SHA from the PR head.
        # Only accept the merged PR whose resulting commit is exactly this release.
        pages = json.loads(output([
            'gh', 'api', '--paginate', '--slurp',
            f'repos/{{owner}}/{{repo}}/commits/{commit}/pulls',
        ]))
        pulls = [pr for page in pages for pr in page
                 if pr.get('merged_at') and pr.get('merge_commit_sha') == commit]
        if len(pulls) == 1:
            run = latest_pr_run(pulls[0]['head']['sha'])
    if run is None:
        raise ValueError('No PR CI run exists for this commit or its merged PR. Run CI on an open PR before releasing.')
    # Wait for pending CI and fail for failed/cancelled runs, including completed ones.
    subprocess.run(['gh', 'run', 'watch', str(run['databaseId']),
                    '--exit-status', '--interval', '15'], cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag', required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--require-ci', action='store_true')
    args = parser.parse_args()
    try:
        version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        validate_tag(args.tag, version, output(['git', 'rev-parse', 'HEAD']), args.commit)
        if args.require_ci:
            require_ci(args.commit)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'release validation: {error}', file=sys.stderr)
        return 1
    print('Release validation passed')
    return 0


if __name__ == '__main__':
    sys.exit(main())
