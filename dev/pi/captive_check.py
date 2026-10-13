#!/usr/bin/env python3
"""Read-only captive DNS/HTTP checks on the Pi through verified Ethernet SSH.

These checks do not prove that a phone displayed its captive sign-in sheet.
"""
import argparse
import ipaddress
import os
import select
import time
from pathlib import Path
import struct
import subprocess

from pi import Pi, ROOT, load_config


def exchange(pi, address, port, payload):
    # The normal remote check verifies Pi identity and Ethernet before -W is used.
    ssh = pi.ssh()
    process = subprocess.Popen([*ssh[:-1], "-W", f"{address}:{port}", ssh[-1]],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        # Keep stdin open until the server closes its response. Closing it with
        # communicate(input=...) can make Dropbear close a forwarded stream
        # before an asynchronously served static file reaches the client.
        process.stdin.write(payload)
        process.stdin.flush()
        chunks = []
        deadline = time.monotonic() + 10
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([process.stdout], [], [], remaining)[0]:
                raise TimeoutError("Pi response timed out")
            chunk = os.read(process.stdout.fileno(), 65536)
            if not chunk:
                break
            chunks.append(chunk)
        response = b"".join(chunks)
        if not response:
            raise RuntimeError("Pi returned no response")
        return response
    finally:
        process.stdin.close()
        process.terminate()
        process.wait(timeout=5)



def http(pi, address, host, path, method="GET"):
    request = f"{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n".encode()
    response = exchange(pi, address, 80, request)
    head = response.split(b"\r\n\r\n", 1)[0].decode()
    lines = head.split("\r\n")
    code = int(lines[0].split()[1])
    headers = dict(line.lower().split(": ", 1) for line in lines[1:] if ": " in line)
    return code, headers


def matches_dns_answer(output, address):
    answer_section = output.split("Name:", 1)[-1]
    answers = [line.split(":", 1)[1].strip() for line in answer_section.splitlines() if line.startswith("Address:")]
    return "captive.apple.com" in answer_section and answers == [address]


def matches_dhcp_config(output, address):
    settings = dict(line.split("=", 1) for line in output.splitlines() if "=" in line)
    return all(settings.get(key) == address for key in ("Address", "Gateway", "DNSList"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / ".local/pi/config.toml")
    parser.add_argument("--ap-ip", default="192.168.16.1")
    args = parser.parse_args()
    address = str(ipaddress.IPv4Address(args.ap_ip))
    pi = Pi(load_config(args.config))
    names = pi.config.get("http_hostnames", [])
    hostname = names[0] if names else pi.remote("exec", "hostname", capture=True).split(".")[0] + ".local"
    hostname = hostname.rstrip(".").lower()
    interface = pi.remote("exec", "ip", "-4", "addr", "show", "dev", pi.config["wifi_interface"], capture=True)
    if f"inet {address}/" not in interface:
        raise RuntimeError("The expected AP address is not assigned to the Pi Wi-Fi interface")
    # Read only network settings, never the profile's password/PSK. This checks
    # configuration; phone lease/router inspection still verifies DHCP delivery.
    profile = pi.remote("exec", "awk",
                        '/^\\[/ { ipv4 = ($0 == "[IPv4]") } '
                        'ipv4 && /^(Address|Gateway|DNSList)=/ { print }',
                        "/var/lib/iwd/ap/wifi-setupd-test.ap", capture=True)
    if not matches_dhcp_config(profile, address):
        raise RuntimeError("AP profile must explicitly set Address, Gateway and DNSList to the AP address")
    print("PASS: AP DHCP profile explicitly configures the local gateway and DNS")
    output = pi.remote("exec", "nslookup", "-type=A", "captive.apple.com", address, capture=True)
    if not matches_dns_answer(output, address):
        raise RuntimeError("AP DNS did not resolve Apple's probe to the AP address")
    print("PASS: UDP DNS resolves captive.apple.com to the AP")
    for kind in (1, 28, 65):
        query = bytes.fromhex("123401000001000000000000") + b"\x07captive\x05apple\x03com\0" + struct.pack("!HH", kind, 1)
        response = exchange(pi, address, 53, struct.pack("!H", len(query)) + query)
        size, = struct.unpack("!H", response[:2])
        answer = response[2:]
        assert len(answer) == size and answer[:2] == query[:2]
        assert answer[3] & 15 == 0
        assert answer[6:8] == (b"\0\1" if kind == 1 else b"\0\0")
        if kind == 1:
            assert answer[-4:] == ipaddress.IPv4Address(address).packed
            assert answer[-10:-6] == bytes(4), "DNS answer must not outlive AP transition"
    print("PASS: TCP DNS supplies zero-TTL IPv4 and empty AAAA/HTTPS answers")
    for host, path in (("captive.apple.com", "/hotspot-detect.html"),
                       ("www.apple.com", "/library/test/success.html"),
                       ("connectivitycheck.gstatic.com", "/generate_204")):
        code, headers = http(pi, address, host, path)
        assert code == 302 and headers["location"] == f"http://{hostname}/"
        assert headers["cache-control"] == "no-store"
    assert http(pi, address, address, "/")[0] == 200
    assert http(pi, address, hostname, "/")[0] == 200
    assert http(pi, address, hostname, "/api/status")[0] == 200
    assert http(pi, address, "captive.apple.com", "/api/status")[0] == 403
    assert http(pi, address, "captive.apple.com", "/api/connect", "POST")[0] == 403
    print(f"PASS: OS probes redirect to http://{hostname}/; hostname HTML/API work; foreign-origin API access remains blocked")
    print("Next: forget/rejoin the AP on the phone and verify that the sign-in sheet opens.")


if __name__ == "__main__":
    main()
