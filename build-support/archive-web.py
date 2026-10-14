#!/usr/bin/env python3
"""Package the tested portal with deterministic metadata and release provenance."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tomllib

from release import ROOT, web_files


def output(command):
    return subprocess.check_output(command, cwd=ROOT, text=True).strip()


def archive_web(web_root, destination, epoch):
    files = web_files(web_root)
    directories = {parent for file in files for parent in file.parents if parent != Path('.')}
    # Construct metadata instead of inheriting host ownership, permissions or times.
    with destination.open('xb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode='w', format=tarfile.PAX_FORMAT) as archive:
                for path in sorted([*directories, *files]):
                    entry = tarfile.TarInfo(path.as_posix())
                    entry.mtime = epoch
                    if path in directories:
                        entry.type = tarfile.DIRTYPE
                        entry.mode = 0o755
                        archive.addfile(entry)
                    else:
                        entry.mode = 0o644
                        entry.size = (web_root / path).stat().st_size
                        with (web_root / path).open('rb') as source:
                            archive.addfile(entry, source)


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--web-root', type=Path, required=True)
    parser.add_argument('--version', help='Release tag; defaults to workspace.package.version')
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'dist')
    parser.add_argument('--require-clean', action='store_true',
                        help='Reject tracked source changes before packaging a release')
    args = parser.parse_args()
    if args.version is None:
        args.version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?', args.version):
        parser.error('Use a numeric release tag such as 0.1.1 or 0.1.1-rc.1 (no v prefix)')
    epoch = int(output(['git', 'show', '-s', '--format=%ct', 'HEAD']))
    provenance = {
        'version': args.version,
        'source_commit': output(['git', 'rev-parse', 'HEAD']),
        'source_dirty': bool(output(['git', 'status', '--porcelain', '--untracked-files=no'])),
        'source_date_epoch': epoch,
        'cargo_lock_sha256': sha256(ROOT / 'Cargo.lock'),
        'tools': {
            name: output([command, '--version'])
            for name, command in {
                'rustc': 'rustc', 'cargo': 'cargo', 'dx': os.environ.get('DX', 'dx'),
                'wasm-bindgen': 'wasm-bindgen', 'wasm-opt': 'wasm-opt', 'esbuild': 'esbuild',
            }.items()
        },
    }
    if args.require_clean and provenance['source_dirty']:
        parser.error('Build changed tracked source files; release packaging requires a clean checkout')
    base = f'wifi-setupd-web-{args.version}'
    archive = args.output_dir / f'{base}.tar.gz'
    metadata = args.output_dir / f'{base}.provenance.json'
    checksums = args.output_dir / f'{base}.sha256'
    if any(path.exists() or path.is_symlink() for path in (archive, metadata, checksums)):
        parser.error('Release output already exists; use a fresh output directory')
    args.output_dir.mkdir(parents=True, exist_ok=True)
    archive_web(args.web_root, archive, epoch)
    provenance['archive_sha256'] = sha256(archive)
    with metadata.open('x') as destination:
        destination.write(json.dumps(provenance, indent=2, sort_keys=True) + '\n')
    with checksums.open('x') as destination:
        for path in (archive, metadata):
            destination.write(f'{sha256(path)}  {path.name}\n')
    print(archive)
    print(checksums)
    print(metadata)


if __name__ == '__main__':
    main()
