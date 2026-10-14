#!/usr/bin/env python3
"""Bump the workspace version interactively and refresh its dependent lockfiles."""
import argparse
from pathlib import Path
import re
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFESTS = ('Cargo.toml', 'dev/portal-sim/Cargo.toml',
             'dev/iwd-probe/Cargo.toml', 'tests/api-consumer/Cargo.toml')


def next_version(version, choice):
    match = re.fullmatch(r'(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?', version)
    if not match:
        raise ValueError(f'Unsupported workspace version: {version}')
    major, minor, patch = map(int, match.groups())
    return {'1': f'{major + 1}.0.0', '2': f'{major}.{minor + 1}.0',
            '3': f'{major}.{minor}.{patch + 1}'}[choice]


def sync_lockfiles(root):
    for manifest in MANIFESTS:
        subprocess.run(['cargo', 'update', '--workspace', '--offline',
                        '--manifest-path', manifest], cwd=root, check=True)


def bump_version(root, new_version):
    manifest = root / 'Cargo.toml'
    paths = [manifest, *(root / Path(p).with_name('Cargo.lock') for p in MANIFESTS)]
    original = {p: p.read_bytes() if p.exists() else None for p in paths}
    source = original[manifest].decode()
    section = re.search(r'(?ms)^\[workspace\.package\][^\n]*\n(.*?)(?=^\[|\Z)', source)
    if not section:
        raise ValueError('Cargo.toml has no workspace.package section')
    updated, count = re.subn(r'''(?m)^(\s*version\s*=\s*)(["'])[^"'\n]+\2''',
                            lambda m: f'{m[1]}{m[2]}{new_version}{m[2]}',
                            section[1], count=1)
    if count != 1:
        raise ValueError('Cannot find workspace.package.version in Cargo.toml')
    try:
        manifest.write_text(source[:section.start(1)] + updated + source[section.end(1):])
        sync_lockfiles(root)
    except (Exception, KeyboardInterrupt):
        # Preserve any edits that existed before this command, including lockfile edits.
        for path, content in original.items():
            if content is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(content)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sync-lockfiles', action='store_true')
    args = parser.parse_args()
    try:
        if args.sync_lockfiles:
            sync_lockfiles(ROOT)
            return 0
        current = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
        print(f'Current version: {current}')
        for choice, label in (('1', 'major'), ('2', 'minor'), ('3', 'patch')):
            print(f'{choice}) {label} -> {next_version(current, choice)}')
        while True:
            try:
                choice = input('Choose 1, 2 or 3 (Enter to cancel): ').strip()
            except EOFError:
                choice = ''
            if not choice:
                print('Cancelled; no files changed.')
                return 0
            if choice in ('1', '2', '3'):
                break
            print('Enter 1, 2 or 3, or leave blank to cancel.')
        new_version = next_version(current, choice)
        bump_version(ROOT, new_version)
        print(f'Updated {current} -> {new_version}; all workspace lockfiles refreshed.')
        return 0
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'version update failed: {error}', file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print('\nCancelled; version bump changes rolled back.', file=sys.stderr)
        return 130


if __name__ == '__main__':
    sys.exit(main())
