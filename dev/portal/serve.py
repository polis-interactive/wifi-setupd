#!/usr/bin/env python3
"""Own the local mock API and dx serve processes for cargo make dev."""
import json
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import tomllib
import urllib.request

from build import ROOT, toolchain


def check_port(port):
    with socket.socket() as listener:
        listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            listener.bind(("127.0.0.1", port))
        except OSError as error:
            raise RuntimeError(f"Port {port} is already in use; stop the previous UI server first.") from error


def stop(process):
    if process.poll() is None:
        try:
            process.terminate()
        except ProcessLookupError:
            return
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    config = Path(sys.argv[1] if len(sys.argv) > 1 else "dev/portal-sim/example.toml").resolve()
    with config.open("rb") as file:
        port = tomllib.load(file).get("port", 8080)
    if type(port) is not int or not 1 <= port <= 65535 or port == 8089:
        raise ValueError("Use a port from 1–65535 other than 8089 (reserved for the mock API).")
    check_port(port)
    check_port(8089)
    dx, env = toolchain()
    children = []
    # Keep children in the terminal group so Ctrl+C reaches both servers even
    # when cargo-make exits before Python finishes cleanup.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    try:
        build = subprocess.Popen(["cargo", "build", "--manifest-path", "dev/portal-sim/Cargo.toml", "--locked", "--offline", "--target-dir", ".target/portal-sim"], cwd=ROOT, env=env)
        children.append(build)
        if build.wait():
            raise RuntimeError("Simulator build failed.")
        backend = subprocess.Popen([str(ROOT / ".target/portal-sim/debug/wifi-setupd-portal-sim"), str(ROOT / ".target/dx/wifi-setupd-ui/debug/web/public"), str(config), "--dev"], cwd=ROOT, env=env)
        children.append(backend)
        for _ in range(100):
            if backend.poll() is not None:
                raise RuntimeError("Simulator exited before becoming ready.")
            try:
                with urllib.request.urlopen("http://127.0.0.1:8089/api/status", timeout=1) as response:
                    if json.load(response)["state"] in ("AwaitingCredentials", "Idle"):
                        break
            except OSError:
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError("Simulator did not become ready within 10 seconds.")
        server = subprocess.Popen([dx, "serve", "--platform", "web", "--package", "wifi-setupd-ui", "--features", "dev", "--addr", "127.0.0.1", "--port", str(port), "--hot-reload", "true", "--watch", "true", "--open", "false", "--interactive", "false", "--locked", "--offline"], cwd=ROOT, env=env)
        children.append(server)
        print(f"UI development: http://127.0.0.1:{port} — Dioxus hot reload enabled; Ctrl+C stops both servers.", flush=True)
        while backend.poll() is None and server.poll() is None:
            time.sleep(0.2)
        raise RuntimeError("A development server exited; see its output above.")
    finally:
        # cargo-make may forward another signal while cleanup is in progress.
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        for child in reversed(children):
            stop(child)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
    except (OSError, ValueError, RuntimeError) as error:
        raise SystemExit(f"dev: {error}") from None
