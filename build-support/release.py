#!/usr/bin/env python3
"""Build product artifacts and install them into a staging root (never a device)."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
WEB_OUTPUT = Path("dx/wifi-setupd-ui/release/web/public")
WEB_SUFFIXES = {".css", ".js", ".wasm", ".svg", ".png", ".ico", ".woff", ".woff2"}


def run(command, env):
    subprocess.run(command, cwd=ROOT, env=env, check=True)


def build(args):
    env = os.environ.copy()
    target_dir = args.target_dir.resolve()
    env["CARGO_TARGET_DIR"] = str(target_dir)
    if args.command == "web":
        # Require preinstalled tools; dx must never download tools during a build.
        env["NO_DOWNLOADS"] = "1"
        public = safe_destination(target_dir, WEB_OUTPUT)
        if public.exists():
            shutil.rmtree(public)  # Only dx's generated product web directory.
        run([args.dx, "build", "--platform", "web", "--release", "--package",
             "wifi-setupd-ui", "--target", "wasm32-unknown-unknown", "--locked",
             *(["--offline"] if args.offline else [])], env)
        web_files(public)
        print(public)
    else:
        run([args.cargo, "build", "--release", "--locked", "--package", "wifi-setupd",
             "--bin", "wifi-setupd", *(["--target", args.target] if args.target else []),
             *(["--offline"] if args.offline else [])], env)


def web_files(root):
    """Validate the entire generated tree before copying any of it."""
    if root.is_symlink() or not root.is_dir():
        raise ValueError(f"Expected a generated web directory: {root}")
    files = []
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root)
        if path.is_symlink():
            raise ValueError(f"Symlink in web output: {relative}")
        if path.is_dir() and relative.parts[0] == "assets":
            continue
        if not path.is_file() or not (
            relative == Path("index.html")
            or (relative.parts[0] == "assets" and path.suffix in WEB_SUFFIXES)
        ):
            raise ValueError(f"Unexpected release web file: {relative}")
        files.append(relative)
    if Path("index.html") not in files or not any(p.suffix == ".wasm" for p in files):
        raise ValueError("Web output must contain index.html and compiled WASM")
    return files


def safe_destination(root, relative):
    path = root
    for part in relative.parts:
        path = path / part
        if path.is_symlink():
            raise ValueError(f"Symlink in install destination: {path}")
    return path


def install(args):
    root = args.destdir.resolve()
    if root == Path("/"):
        raise ValueError("--destdir must be a staging directory, not /")
    files = web_files(args.web_root)
    if args.binary.is_symlink():
        raise ValueError("The daemon binary must not be a symlink")
    with args.binary.open("rb") as binary:
        if binary.read(4) != b"\x7fELF":
            raise ValueError("The daemon binary must be a Linux ELF executable")
    binary_dest = safe_destination(root, Path("usr/bin/wifi-setupd"))
    web_dest = safe_destination(root, Path("usr/share/wifi-setupd/web"))
    license_dest = safe_destination(root, Path("usr/share/licenses/wifi-setupd/LICENSE-MIT"))
    # Reject overlapping inputs before refreshing the package-owned web subtree.
    for source in (args.web_root.resolve(), args.binary.resolve(), ROOT / "LICENSE-MIT"):
        if source == web_dest or web_dest in source.parents or source in web_dest.parents:
            raise ValueError("Install inputs must be outside the installed web directory")
    if web_dest.exists():
        shutil.rmtree(web_dest)
    for source, destination, mode in [
        (args.binary, binary_dest, 0o755),
        (ROOT / "LICENSE-MIT", license_dest, 0o644),
        *((args.web_root / p, web_dest / p, 0o644) for p in files),
    ]:
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(mode)
    print(f"Installed daemon, {len(files)} web files and license into {root}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("web", "daemon"):
        command = commands.add_parser(name)
        command.add_argument("--target-dir", type=Path,
                             default=Path(os.environ.get("CARGO_TARGET_DIR", ROOT / ".target")))
        command.add_argument("--offline", action="store_true")
        if name == "web":
            command.add_argument("--dx", default=os.environ.get("DX", "dx"))
        else:
            command.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"))
            command.add_argument("--target", default=os.environ.get("CARGO_BUILD_TARGET"))
    command = commands.add_parser("install")
    command.add_argument("--binary", type=Path, required=True)
    command.add_argument("--web-root", type=Path, required=True)
    command.add_argument("--destdir", type=Path, required=True)
    args = parser.parse_args()
    try:
        install(args) if args.command == "install" else build(args)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
