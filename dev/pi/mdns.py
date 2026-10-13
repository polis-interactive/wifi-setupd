#!/usr/bin/env python3
"""Apply BuildrootImages' development Avahi config for this boot over Ethernet."""
import argparse
import hashlib
from pathlib import Path

from pi import Pi, ROOT, load_config
from portal import remote_input

# The image stays read-only. The exact image-owned file is bind-mounted until
# reboot; restart failure unmounts it and restores the original Avahi service.
INSTALL = '''set -eu
expected=$1
target=/etc/avahi/avahi-daemon.conf
service=/etc/init.d/S50avahi-daemon
stage=$(mktemp -d /run/wifi-setupd-avahi.XXXXXX)
chmod 755 "$stage"
cat >"$stage/avahi-daemon.conf"
chmod 644 "$stage/avahi-daemon.conf"
actual=$(sha256sum "$stage/avahi-daemon.conf"); actual=${actual%% *}
[ "$actual" = "$expected" ] || { echo 'Incomplete Avahi configuration upload' >&2; exit 1; }
if cmp -s "$target" "$stage/avahi-daemon.conf"; then
    "$service" start
    echo 'Matching Avahi configuration already installed'
    exit 0
fi
"$service" stop
if ! mount -o bind "$stage/avahi-daemon.conf" "$target"; then
    "$service" start
    exit 1
fi
if ! "$service" start; then
    umount "$target"
    "$service" start
    echo 'Avahi rejected update; restored previous configuration' >&2
    exit 1
fi
sha256sum "$target"
echo 'Avahi updated for this boot; reboot removes the bind mount'
'''


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=ROOT / ".local/pi/config.toml")
    args = parser.parse_args()
    pi = Pi(load_config(args.config))
    source = Path(pi.config["buildroot_images"]) / "profiles/groups/development-access/rootfs-overlay/etc/avahi/avahi-daemon.conf"
    data = source.read_bytes()
    remote_input(pi, INSTALL, data, hashlib.sha256(data).hexdigest())


if __name__ == "__main__":
    main()
