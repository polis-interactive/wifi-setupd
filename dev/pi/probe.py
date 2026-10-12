#!/usr/bin/env python3
"""Build and run the isolated iwd adapter probe over verified Ethernet SSH."""
import argparse
import getpass
import hashlib
import os
from pathlib import Path
import subprocess

from pi import Pi, ROOT, load_config, validate_elf

MANIFEST = ROOT / "dev/iwd-probe/Cargo.toml"
REMOTE_PROBE = "/data/bin/wifi-setupd-iwd-probe"
UPLOAD = '''set -eu
app=$1 expected=$2 size=$3
mkdir -p "$(dirname "$app")"
exec 9>"$app.update.lock"
/bin/busybox flock -n 9
stage="$app.upload.$$"
trap 'rm -f "$stage"' EXIT
trap 'exit 1' HUP INT TERM
umask 077
cat >"$stage"
[ "$(wc -c <"$stage")" -eq "$size" ]
actual=$(sha256sum "$stage"); actual=${actual%% *}
[ "$actual" = "$expected" ]
chmod 700 "$stage"
mv -f "$stage" "$app"
sha256sum "$app"
'''

# iwd owns the AP configuration format. Keep this temporary hardware workaround
# in the development suite; the product adapter only invokes StartProfile.
AP_PROFILE = '''set -eu
umask 077
mkdir -p /var/lib/iwd/ap
profile=/var/lib/iwd/ap/$3.ap
# Refuse to overwrite any existing iwd AP profile, including symlinks.
(set -C; : >"$profile") || exit 1
trap 'rm -f "$profile"' EXIT
trap 'exit 1' HUP INT TERM
cat >"$profile"
"$1" "$2" ap-profile "$3"
'''


def run_ap(pi, remote_probe, ssid, password):
    if not 1 <= len(ssid.encode()) <= 32 or any(c in ssid for c in "/\x00\r\n"):
        raise ValueError("AP SSID must be 1–32 UTF-8 bytes without slashes or line breaks")
    if not 8 <= len(password.encode()) <= 63 or any(c in password for c in "\x00\r\n"):
        raise ValueError("AP passphrase must be 8–63 bytes without line breaks")
    # Hex PSK avoids settings-file escaping for user-supplied passphrases.
    psk = hashlib.pbkdf2_hmac("sha1", password.encode(), ssid.encode(), 4096, 32).hex()
    profile = f"[General]\nDisableHT=true\n[Security]\nPreSharedKey={psk}\n[IPv4]\n".encode()
    remote = pi.command("exec", ["sh", "-c", AP_PROFILE, "probe-ap", remote_probe,
                                 pi.config["wifi_interface"], ssid])
    subprocess.run([*pi.ssh(), remote], input=profile, timeout=240, check=True)


def run(pi, remote_probe, command, args, secret=None):
    remote = pi.command("exec", [remote_probe, pi.config["wifi_interface"], command, *args])
    # subprocess input uses an anonymous pipe. No secret file or shell expansion.
    subprocess.run([*pi.ssh(), remote], input=secret, timeout=240, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / "local/pi/config.toml")
    parser.add_argument("--remote-probe", default=REMOTE_PROBE)
    parser.add_argument("command", choices=["build", "deploy", "status", "monitor", "scan", "known", "connect", "replace", "connect-known", "forget", "disconnect", "ap", "stop-ap"])
    parser.add_argument("args", nargs="*")
    options = parser.parse_args()
    pi = Pi(load_config(options.config))
    if not options.remote_probe.startswith("/") or "\n" in options.remote_probe or "\x00" in options.remote_probe:
        parser.error("remote-probe must be an absolute path")
    artifact = ROOT / "target/iwd-probe" / pi.config["target"] / "release/wifi-setupd-iwd-probe"
    if options.command == "build":
        env = os.environ.copy()
        env["CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"] = pi.config["linker"]
        subprocess.run(["cargo", "build", "--manifest-path", str(MANIFEST), "--locked", "--offline", "--release", "--target", pi.config["target"], "--target-dir", str(ROOT / "target/iwd-probe")], env=env, check=True)
    elif options.command == "deploy":
        validate_elf(artifact)
        with artifact.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
            size = os.fstat(source.fileno()).st_size
            source.seek(0)
            pi.remote("exec", "sh", "-c", UPLOAD, "probe-upload", options.remote_probe, digest, size, stdin=source, timeout=180)
    elif options.command in ("connect", "replace", "ap"):
        ssid = options.args[0] if options.args else input("SSID: ")
        if options.command == "replace":
            confirmation = input(f"Discard the saved profile for {ssid!r} and replace its credentials? Type replace: ")
            if confirmation != "replace":
                raise ValueError("Replacement cancelled")
        prompt = "Temporary Pi AP passphrase (hidden): " if options.command == "ap" else "Router passphrase (hidden): "
        password = getpass.getpass(prompt)
        if any("\n" in value or "\r" in value for value in (ssid, password)):
            raise ValueError("SSID/password cannot contain a newline")
        if options.command == "ap":
            run_ap(pi, options.remote_probe, ssid, password)
        else:
            run(pi, options.remote_probe, options.command, [], (ssid + "\n" + password + "\n").encode())
    else:
        run(pi, options.remote_probe, options.command, options.args)


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        raise SystemExit(f"probe: command failed with exit status {error.returncode}") from None
    except subprocess.TimeoutExpired:
        raise SystemExit("probe: command timed out; check Pi status and run scoped cleanup") from None
    except (ValueError, OSError, RuntimeError, subprocess.SubprocessError) as error:
        raise SystemExit(f"probe: {error}") from None
