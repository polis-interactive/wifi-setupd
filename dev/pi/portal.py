#!/usr/bin/env python3
"""Build/deploy/run the development portal over verified Ethernet SSH."""
import argparse
import hashlib
import io
import os
from pathlib import Path
import secrets
import subprocess
import tarfile

from pi import Pi, ROOT, load_config, validate_elf
import device_key

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
# The installed development image has a read-only /etc and predates this
# service. Preserve its configuration in a temporary bind mount and grant only
# root access to this exact bus name. Reboot removes the mount; image-owned
# production policy belongs in BuildrootImages.
DBUS_POLICY = '''set -eu
[ -f "$1/device-key" ] || { echo 'Run portal.py provision first' >&2; exit 1; }
policy=/etc/dbus-1/system.d/wifi-setupd-development.conf
if [ ! -f "$policy" ]; then
    stage=$(mktemp -d /run/wifi-setupd-dbus.XXXXXX)
    cp -a /etc/dbus-1/. "$stage/"
    mkdir -p "$stage/system.d"
    chmod 755 "$stage" "$stage/system.d"
    cat >"$stage/system.d/wifi-setupd-development.conf" <<'POLICY'
<busconfig>
  <policy user="root">
    <allow own="tv.polis.WifiSetup1"/>
    <allow send_destination="tv.polis.WifiSetup1"/>
  </policy>
</busconfig>
POLICY
    chmod 644 "$stage/system.d/wifi-setupd-development.conf"
    mount -o bind "$stage" /etc/dbus-1
fi
dbus-send --system --type=method_call --print-reply --dest=org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus.ReloadConfig >/dev/null
'''
START = '''set -eu
root=$1 interface=$2
shift 2
umask 077
mkdir -p "$root"
exec 9>"$root/provision.lock"
/bin/busybox flock -n 9
[ ! -e "$root/provisioning-incomplete" ] || { echo 'Rerun portal.py provision to finish the password update' >&2; exit 1; }
[ -f "$root/device-key" ] || { echo 'Run portal.py provision first' >&2; exit 1; }
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
nohup "$root/current/wifi-setupd" --interface "$interface" --ap-profile wifi-setupd-test --web-root "$root/current/web" --device-key-file "$root/device-key" "$@" 9>&- >"$root/log" 2>&1 </dev/null &
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
    parser.add_argument("command", choices=["build", "deploy", "provision", "start", "stop", "status", "logs"])
    parser.add_argument("--config", type=Path, default=ROOT / ".local/pi/config.toml")
    local_dx = ROOT / ".local/tools/bin/dx"
    parser.add_argument("--dx", default=str(local_dx) if local_dx.exists() else "dx")
    keys = parser.add_mutually_exclusive_group()
    keys.add_argument("--key-file", type=Path, help="Import a device password during provisioning")
    keys.add_argument("--rotate-key", action="store_true", help="Generate a new device password during provisioning")
    args = parser.parse_args()
    if args.command != "provision" and (args.key_file is not None or args.rotate_key):
        parser.error("Key options require the provision command")
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
        print("Portal binary and offline web assets deployed. Run: python3 dev/pi/portal.py start")
    elif args.command == "provision":
        directory = ROOT / ".local/devices" / pi.config["expected_serial"]
        key_path = device_key.provision(pi, remote_input, REMOTE, AP_NAME, directory,
                                       args.key_file, args.rotate_key)
        print(f"Device password installed for AP access and saved-network management. Local copy: {key_path}")
        print("Run: python3 dev/pi/portal.py start")
    elif args.command == "start":
        hostnames = pi.config.get("http_hostnames", [])
        if not isinstance(hostnames, list) or not all(isinstance(name, str) for name in hostnames):
            raise ValueError("http_hostnames must be an array of DNS names")
        hostname_args = [argument for name in hostnames for argument in ("--http-hostname", name)]
        pi.remote("exec", "sh", "-c", DBUS_POLICY, "portal-policy", REMOTE)
        pi.remote("exec", "sh", "-c", START, "portal", REMOTE, pi.config["wifi_interface"], *hostname_args)
        print("Portal started. When Wi-Fi is disconnected, join wifi-setupd-test for setup; the AP has no shutdown timer.")
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
