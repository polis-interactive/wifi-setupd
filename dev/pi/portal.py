#!/usr/bin/env python3
"""Build/deploy/run the development portal over verified Ethernet SSH."""
import argparse
import getpass
import hashlib
import io
import os
from pathlib import Path
import secrets
import subprocess
import tarfile

from pi import Pi, ROOT, load_config, validate_elf

REMOTE = "/data/wifi-setupd-portal"
AP_NAME = "wifi-setupd-test"
WEB = ROOT / ".target/dx/wifi-setupd-ui/release/web/public"

UPLOAD = '''set -eu
root=$1 release=$2 expected=$3
umask 077
mkdir -p "$root/releases"
stage="$root/releases/$release"
mkdir "$stage"
trap 'rm -rf "$stage"' EXIT
cat >"$stage/bundle.tar"
actual=$(sha256sum "$stage/bundle.tar"); actual=${actual%% *}
[ "$actual" = "$expected" ]
tar -xf "$stage/bundle.tar" -C "$stage"
rm "$stage/bundle.tar"
chmod 700 "$stage/wifi-setupd"
ln -s "releases/$release" "$root/current.new"
mv -Tf "$root/current.new" "$root/current"
trap - EXIT
'''
PROFILE = '''set -eu
umask 077
mkdir -p /var/lib/iwd/ap
profile=/var/lib/iwd/ap/wifi-setupd-test.ap
# Explicit development fixture: never overwrite an existing iwd profile.
(set -C; : >"$profile")
trap 'rm -f "$profile"' EXIT
cat >"$profile"
trap - EXIT
'''
START = '''set -eu
root=$1 interface=$2
[ -x "$root/current/wifi-setupd" ]
[ -f "$root/current/web/index.html" ]
[ -f /var/lib/iwd/ap/wifi-setupd-test.ap ]
if [ -f "$root/pid" ]; then
    pid=$(cat "$root/pid")
    if kill -0 "$pid" 2>/dev/null; then
        case "$(readlink /proc/$pid/exe)" in "$root"/releases/*/wifi-setupd)
            echo 'Portal already running; stop it before starting a new build'; exit 0;;
        esac
    fi
fi
umask 077
nohup "$root/current/wifi-setupd" --interface "$interface" --ap-profile wifi-setupd-test --web-root "$root/current/web" --management-key-file "$root/management-key" >"$root/log" 2>&1 </dev/null &
echo $! >"$root/pid"
sleep 2
kill -0 "$(cat "$root/pid")" || { cat "$root/log"; exit 1; }
cat "$root/log"
'''
STOP = '''set -eu
root=$1
[ -f "$root/pid" ] || exit 0
pid=$(cat "$root/pid")
exe=$(readlink "/proc/$pid/exe" || true)
case "$exe" in "$root"/releases/*/wifi-setupd) kill -TERM "$pid";; *) echo 'Portal is not running'; exit 0;; esac
# Wait for daemon cleanup, not an AP lifetime timer.
for n in $(seq 1 35); do kill -0 "$pid" 2>/dev/null || exit 0; sleep 1; done
echo 'Daemon is still reconciling; inspect logs before restarting' >&2
exit 1
'''


def remote_input(pi, script, payload, *args):
    subprocess.run([*pi.ssh(), pi.command("exec", ["sh", "-c", script, "portal", *args])], input=payload, check=True, timeout=180)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["build", "deploy", "start", "stop", "status", "logs"])
    parser.add_argument("--config", type=Path, default=ROOT / ".local/pi/config.toml")
    local_dx = ROOT / ".local/tools/bin/dx"
    parser.add_argument("--dx", default=str(local_dx) if local_dx.exists() else "dx")
    args = parser.parse_args()
    pi = Pi(load_config(args.config))
    artifact = ROOT / ".target" / pi.config["target"] / "release/wifi-setupd"
    if args.command == "build":
        env = os.environ.copy()
        tools = ROOT / ".local/tools"
        if all((tools / "bin" / name).is_file() for name in ("wasm-bindgen", "wasm-opt", "esbuild")):
            env["PATH"] = str(tools / "bin") + os.pathsep + env.get("PATH", "")
            env["LD_LIBRARY_PATH"] = str(tools / "lib") + os.pathsep + env.get("LD_LIBRARY_PATH", "")
            env["NO_DOWNLOADS"] = "1"
        subprocess.run([args.dx, "build", "--platform", "web", "--release", "--package", "wifi-setupd-ui", "--locked", "--offline"], cwd=ROOT, env=env, check=True)
        env["CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"] = pi.config["linker"]
        subprocess.run(["cargo", "build", "-p", "wifi-setupd", "--release", "--locked", "--offline", "--target", pi.config["target"]], cwd=ROOT, env=env, check=True)
    elif args.command == "deploy":
        validate_elf(artifact)
        if not (WEB / "index.html").is_file():
            raise ValueError("Build web assets first")
        bundle = io.BytesIO()
        with tarfile.open(fileobj=bundle, mode="w") as archive:
            archive.add(artifact, arcname="wifi-setupd")
            archive.add(WEB, arcname="web")
        payload = bundle.getvalue()
        digest = hashlib.sha256(payload).hexdigest()
        # Unique release directories keep the currently executing binary intact.
        remote_input(pi, UPLOAD, payload, REMOTE, digest[:12] + "-" + secrets.token_hex(4), digest)
        key_path = ROOT / ".local/pi/portal-management-key"
        if not key_path.exists():
            with open(key_path, "x", opener=lambda p, flags: os.open(p, flags, 0o600)) as file:
                file.write(secrets.token_hex(32) + "\n")
        remote_input(pi, 'set -eu; umask 077; cat >"$1/management-key"', key_path.read_bytes(), REMOTE)
        print("Portal binary and offline web assets deployed. Run: python3 dev/pi/portal.py start")
    elif args.command == "start":
        exists = pi.remote("exec", "sh", "-c", "if [ -e /var/lib/iwd/ap/wifi-setupd-test.ap ]; then echo yes; fi", capture=True)
        if exists != "yes":
            password = getpass.getpass("Pi setup AP passphrase (not your router password; hidden): ")
            if not 8 <= len(password) <= 63 or not password.isascii() or any(c in password for c in "\x00\r\n"):
                raise ValueError("Use 8–63 ASCII characters without line breaks")
            psk = hashlib.pbkdf2_hmac("sha1", password.encode(), AP_NAME.encode(), 4096, 32).hex()
            # Runtime test fixture only. Production image configuration belongs
            # in BuildrootImages. Wi-Fi secrets persist only inside iwd storage.
            profile = f"[General]\nDisableHT=true\n[Security]\nPreSharedKey={psk}\n[IPv4]\nAddress=192.168.16.1\nNetmask=255.255.255.240\n"
            remote_input(pi, PROFILE, profile.encode())
        pi.remote("exec", "sh", "-c", START, "portal", REMOTE, pi.config["wifi_interface"])
        print("Join wifi-setupd-test and open http://192.168.16.1 . The AP has no shutdown timer.")
    elif args.command == "stop":
        pi.remote("exec", "sh", "-c", STOP, "portal", REMOTE, timeout=45)
    elif args.command == "logs":
        pi.remote("exec", "tail", "-n", "40", REMOTE + "/log")
    else:
        pi.remote("exec", "sh", "-c", 'ip -4 addr show dev "$1"; tail -n 10 "$2/log"', "portal", pi.config["wifi_interface"], REMOTE)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"portal: {error}") from None
