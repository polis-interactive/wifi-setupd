#!/usr/bin/env python3
"""Development-only Pi commands. Requires Python 3.11+ and OpenSSH."""

import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import sys
import time
import tomllib
import uuid

ROOT = Path(__file__).resolve().parents[2]
REMOTE = Path(__file__).with_name("remote.sh")


def load_config(path):
    with path.open("rb") as source:
        config = tomllib.load(source)
    required = (
        "host", "port", "user", "identity_file", "known_hosts", "expected_serial",
        "ethernet_interface", "wifi_interface", "host_wifi_interface", "buildroot_images", "linker",
        "target", "remote_app", "remote_service", "remote_log",
        "command_timeout", "reboot_timeout",
    )
    for key in required:
        if key not in config:
            raise ValueError(f"Missing configuration: {key}")
    for key in ("host", "user", "ethernet_interface", "wifi_interface", "host_wifi_interface"):
        if not re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.:-]*", config[key]):
            raise ValueError(f"Invalid {key}")
    if not re.fullmatch(r"[0-9a-fA-F]{16}", config["expected_serial"]):
        raise ValueError("Set expected_serial to the verified Pi serial")
    for key in ("remote_app", "remote_service", "remote_log"):
        value = config[key]
        if not value.startswith("/") or "\n" in value or "\x00" in value:
            raise ValueError(f"{key} must be an absolute remote path")
    for key in ("command_timeout", "reboot_timeout", "port"):
        if type(config[key]) is not int or config[key] <= 0:
            raise ValueError(f"{key} must be a positive integer")
    if config["port"] > 65535:
        raise ValueError("Invalid SSH port")
    if config["target"] != "aarch64-unknown-linux-musl":
        raise ValueError("This image workflow currently supports aarch64-unknown-linux-musl")
    for key in ("identity_file", "known_hosts", "buildroot_images", "linker"):
        # Buildroot's compiler wrapper uses the symlink's invocation name.
        config[key] = os.path.abspath(ROOT / Path(config[key]).expanduser())
    return config


class Pi:
    def __init__(self, config):
        self.config = config

    def ssh(self, interactive=False):
        c = self.config
        return [
            "ssh", "-4", "-tt" if interactive else "-T", "-p", str(c["port"]),
            "-i", c["identity_file"], "-o", "IdentitiesOnly=yes",
            "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
            "-o", f"UserKnownHostsFile={c['known_hosts']}",
            "-o", "ConnectTimeout=5", "-o", "ConnectionAttempts=1",
            "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2",
            f"{c['user']}@{c['host']}",
        ]

    def command(self, action, args):
        c = self.config
        return shlex.join([
            "sh", "-c", REMOTE.read_text(), "wifi-setupd-dev", action,
            c["expected_serial"], c["ethernet_interface"], c["remote_app"],
            c["remote_service"], c["remote_log"], *map(str, args),
        ])

    def remote(self, action, *args, capture=False, stdin=None, timeout=None, interactive=False):
        result = subprocess.run(
            [*self.ssh(interactive), self.command(action, args)],
            stdin=stdin if stdin is not None else (None if interactive else subprocess.DEVNULL),
            stdout=subprocess.PIPE if capture else None,
            stderr=subprocess.PIPE if capture else None,
            timeout=timeout or self.config["command_timeout"], check=False,
        )
        if result.returncode:
            detail = (result.stderr or b"").decode(errors="replace").strip()
            raise RuntimeError(f"Remote {action} failed ({result.returncode}): {detail}")
        return (result.stdout or b"").decode(errors="replace").strip()

    def build(self):
        c = self.config
        if not Path(c["linker"]).is_file():
            raise ValueError(f"Cross-linker does not exist: {c['linker']}")
        env = os.environ.copy()
        env["CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"] = c["linker"]
        subprocess.run([
            "cargo", "build", "--locked", "--release", "-p", "wifi-setupd",
            "--target", c["target"],
        ], cwd=ROOT, env=env, check=True)
        print(ROOT / ".target" / c["target"] / "release/wifi-setupd")

    def deploy(self, artifact):
        validate_elf(artifact)
        with artifact.open("rb") as source:
            digest = hashlib.file_digest(source, "sha256").hexdigest()
            # file_digest does not guarantee the stream position.
            size = os.fstat(source.fileno()).st_size
            source.seek(0)
            self.remote("deploy", digest, size, stdin=source, timeout=180)

    def reboot(self):
        before = self.remote("exec", "cat", "/proc/sys/kernel/random/boot_id", capture=True)
        machine = self.remote("exec", "cat", "/var/lib/dbus/machine-id", capture=True)
        token = uuid.uuid4().hex
        marker = f"/var/lib/iwd/.wifi-setupd-persistence-{token}"
        self.remote("exec", "sh", "-c", 'umask 077; printf "%s\\n" "$1" >"$2"; sync', "sh", token, marker)
        print("Reboot requested; waiting for a new boot ID over Ethernet...", flush=True)
        deadline = time.monotonic() + self.config["reboot_timeout"]
        # Send once. A disconnect is ambiguous: observe, never replay reboot.
        try:
            self.remote("exec", "reboot", capture=True, timeout=10)
        except (RuntimeError, subprocess.TimeoutExpired):
            pass
        while time.monotonic() < deadline:
            try:
                boot = self.remote("exec", "cat", "/proc/sys/kernel/random/boot_id", capture=True, timeout=8)
            except (RuntimeError, subprocess.TimeoutExpired):
                time.sleep(2)
                continue
            if boot != before:
                saved = self.remote("exec", "cat", marker, capture=True)
                new_machine = self.remote("exec", "cat", "/var/lib/dbus/machine-id", capture=True)
                if saved != token or new_machine != machine:
                    raise RuntimeError("Reboot completed but persistence check failed")
                self.remote("exec", "rm", marker)
                print(json.dumps({"before": before, "after": boot, "persistence": "passed"}))
                return
            time.sleep(2)
        raise RuntimeError("Reboot not confirmed before deadline; query status before retrying")


def validate_elf(path):
    with path.open("rb") as source:
        header = source.read(64)
        if len(header) != 64 or header[:6] != b"\x7fELF\x02\x01":
            raise ValueError("Expected a 64-bit little-endian ELF")
        if int.from_bytes(header[18:20], "little") != 183:
            raise ValueError("Expected AArch64 ELF; refusing a native laptop binary")
        offset = int.from_bytes(header[32:40], "little")
        size = int.from_bytes(header[54:56], "little")
        count = int.from_bytes(header[56:58], "little")
        if size < 56 or count < 1:
            raise ValueError("Invalid ELF program headers")
        for index in range(count):
            source.seek(offset + index * size)
            segment = source.read(size)
            if len(segment) != size:
                raise ValueError("Truncated ELF")
            segment_type = int.from_bytes(segment[:4], "little")
            if segment_type == 3:
                raise ValueError("Development image requires a static executable")
            if segment_type == 2:
                # Static PIEs have PT_DYNAMIC too; reject actual shared dependencies.
                dynamic_offset = int.from_bytes(segment[8:16], "little")
                dynamic_size = int.from_bytes(segment[32:40], "little")
                source.seek(dynamic_offset)
                for _ in range(dynamic_size // 16):
                    entry = source.read(16)
                    if len(entry) != 16:
                        raise ValueError("Truncated ELF dynamic table")
                    tag = int.from_bytes(entry[:8], "little")
                    if tag == 0:
                        break
                    if tag == 1:
                        raise ValueError("Development image requires a static executable")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / ".local/pi/config.toml")
    commands = parser.add_subparsers(dest="action", required=True)
    for action in ("status", "check", "build", "run", "start", "stop", "logs", "rollback", "reboot", "wifi-status", "wifi-ping", "ssh"):
        commands.add_parser(action)
    deploy = commands.add_parser("deploy")
    deploy.add_argument("artifact", type=Path)
    execute = commands.add_parser("exec")
    execute.add_argument("command", nargs=argparse.REMAINDER)
    connect = commands.add_parser("wifi-connect", help="Interactive iwctl password prompt over Ethernet SSH")
    connect.add_argument("ssid", nargs="?")
    args = parser.parse_args()
    pi = Pi(load_config(args.config))
    if args.action == "ssh":
        # Keep the verified remote wrapper, but let SSH own the terminal and
        # session lifetime instead of imposing the remote-command timeout.
        os.execvp("ssh", [*pi.ssh(interactive=True), pi.command("exec", ["sh", "-il"])])
    elif args.action == "build":
        pi.build()
    elif args.action == "deploy":
        pi.deploy(args.artifact.resolve())
    elif args.action == "reboot":
        pi.reboot()
    elif args.action == "run":
        pi.remote("exec", pi.config["remote_app"])
    elif args.action == "exec":
        if not args.command:
            parser.error("exec needs a command")
        pi.remote("exec", *args.command)
    elif args.action == "wifi-status":
        pi.remote("exec", "iwctl", "station", pi.config["wifi_interface"], "show")
        pi.remote("exec", "ip", "-4", "addr", "show", "dev", pi.config["wifi_interface"])
    elif args.action == "wifi-connect":
        if not sys.stdin.isatty():
            raise ValueError("Run wifi-connect in your terminal for the private password prompt")
        ssid = args.ssid if args.ssid is not None else input("Wi-Fi SSID: ")
        if not ssid or len(ssid.encode()) > 32 or any(c in ssid for c in "\r\n\x00"):
            raise ValueError("SSID must be 1–32 UTF-8 bytes without line breaks")
        pi.remote("exec", "iwctl", "station", pi.config["wifi_interface"], "connect", ssid,
                  interactive=True, timeout=180)
        print("Credentials remain on the Pi until you explicitly forget the network.")
    elif args.action == "wifi-ping":
        addresses = pi.remote("exec", "ip", "-o", "-4", "addr", "show", "dev",
                              pi.config["wifi_interface"], "scope", "global", capture=True)
        candidates = re.findall(r"\binet ([0-9.]+)/", addresses)
        if len(candidates) != 1:
            raise ValueError("Expected one Pi Wi-Fi IPv4 address; run wifi-status")
        address = str(ipaddress.IPv4Address(candidates[0]))
        print(f"Pinging Pi Wi-Fi address {address} via {pi.config['host_wifi_interface']}", flush=True)
        subprocess.run(["ping", "-I", pi.config["host_wifi_interface"], "-c", "3",
                        "-W", "2", address], check=True, timeout=15)
    else:
        pi.remote(args.action)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"pi: {error}", file=sys.stderr)
        sys.exit(1)
