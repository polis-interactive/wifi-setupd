#!/usr/bin/env python3
"""Build UI assets without reading Pi configuration or contacting hardware."""
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]


def toolchain():
    env = os.environ.copy()
    tools = ROOT / ".local/tools"
    local_dx = tools / "bin/dx"
    dx = env.get("DX", str(local_dx) if local_dx.is_file() else "dx")
    if all((tools / "bin" / name).is_file() for name in ("wasm-bindgen", "wasm-opt", "esbuild")):
        env["PATH"] = str(tools / "bin") + os.pathsep + env.get("PATH", "")
        env["LD_LIBRARY_PATH"] = str(tools / "lib") + os.pathsep + env.get("LD_LIBRARY_PATH", "")
        env["NO_DOWNLOADS"] = "1"
    return dx, env


def main():
    dx, env = toolchain()
    subprocess.run([sys.executable, "build-support/release.py", "web", "--dx", dx,
                    "--offline"], cwd=ROOT, env=env, check=True)


if __name__ == "__main__":
    main()
