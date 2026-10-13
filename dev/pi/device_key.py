"""Per-device AP/management secrets for development provisioning, never builds."""
import hashlib
import io
import os
from pathlib import Path
import secrets
import tarfile

# Short, familiar words keep the per-device password practical to type on a phone.
WORDS = tuple("""
acorn apple apron arrow atlas badge bagel baker bamboo basil beach berry
birch bird bloom boat boots bowl bread brick brook brush cabin cable camel
candle candy canoe cedar chair cherry chest chili cloud clover coast cocoa
comet coral crane creek crown daisy dawn deer delta dove dream drum dune
eagle earth elm ember fern field finch flame flower flute forest fox frost
garden gate gecko glass globe grape grass grove gull harbor hazel heron hill
honey horse house ivy jade kite kiwi lake lamp lark leaf lemon light lilac
lily lime lion lotus maple marble meadow melon mint moon moss mouse nest
oak ocean olive otter owl panda paper peach pearl pebble pine plum pond
poppy porch prism quilt rabbit raven reef river robin rock rose sail sand
seed shell shore sky slate snow soap solar sparrow spice spoon spring star
stone storm sugar sunny swan table tea thyme tiger toast trail tree tulip
valley vine violet water wave wheat willow wind wing wolf wood wren zebra
""".split())


def generate():
    """Three lowercase words followed by exactly three random decimal digits."""
    return "".join(secrets.choice(WORDS) for _ in range(3)) + f"{secrets.randbelow(1000):03d}"


# Complete both writes before allowing the portal to start. Failed/uncertain
# installs leave a marker; rerunning provisioning with the pending key repairs it.
INSTALL = '''set -eu
root=$1 interface=$2 profile=$3 expected=$4
umask 077
mkdir -p "$root" "$(dirname "$profile")"
exec 9>"$root/provision.lock"
/bin/busybox flock -n 9
if [ -f "$root/pid" ]; then
    pid=$(cat "$root/pid")
    case "$(readlink /proc/$pid/exe || true)" in "$root"/releases/*/wifi-setupd)
        echo 'Stop the portal before provisioning its device password' >&2; exit 1;;
    esac
fi
stage=$(mktemp -d "$root/provision.XXXXXX")
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
cat >"$stage/bundle.tar"
actual=$(sha256sum "$stage/bundle.tar"); actual=${actual%% *}
[ "$actual" = "$expected" ] || { echo 'Incomplete provisioning upload' >&2; exit 1; }
tar -xf "$stage/bundle.tar" -C "$stage"
# Drop a previously active AP so the next StartProfile reads the new PSK.
# This runs only for explicit provisioning, over verified Ethernet.
iwctl device "$interface" set-property Mode station
: >"$root/provisioning-incomplete"
sync
cp "$stage/device-key" "$root/device-key.new"
chmod 600 "$root/device-key.new"
mv -f "$root/device-key.new" "$root/device-key"
cp "$stage/ap.profile" "$profile.new"
chmod 600 "$profile.new"
mv -f "$profile.new" "$profile"
sync
rm "$root/provisioning-incomplete"
sync
'''


def validate(value):
    if not 8 <= len(value) <= 63 or not all(33 <= ord(c) <= 126 for c in value):
        raise ValueError("Device password must be 8–63 printable ASCII characters without spaces")
    return value


def read_key(path):
    return validate(path.read_text().rstrip("\r\n"))


def prepare(directory, source=None, rotate=False):
    """Return a persistent candidate; retries reuse it until installation succeeds."""
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory.chmod(0o700)
    current = directory / "device-key"
    pending = directory / "device-key.pending"
    if pending.exists():
        if source is not None or rotate:
            raise ValueError("A device password is pending; rerun provision without key options to finish it")
        read_key(pending)
        return pending
    if source is None and not rotate and current.exists():
        read_key(current)
        return current
    value = read_key(source) if source is not None else generate()
    with open(pending, "x", opener=lambda p, flags: os.open(p, flags, 0o600)) as file:
        file.write(value + "\n")
        file.flush()
        os.fsync(file.fileno())
    return pending


def payload(ssid, key):
    validate(key)
    psk = hashlib.pbkdf2_hmac("sha1", key.encode(), ssid.encode(), 4096, 32).hex()
    profile = (f"[General]\nDisableHT=true\n[Security]\nPreSharedKey={psk}\n"
               "[IPv4]\nAddress=192.168.16.1\nNetmask=255.255.255.240\n"
               "Gateway=192.168.16.1\nDNSList=192.168.16.1\n")
    bundle = io.BytesIO()
    with tarfile.open(fileobj=bundle, mode="w") as archive:
        for name, value in (("device-key", key + "\n"), ("ap.profile", profile)):
            data = value.encode()
            info = tarfile.TarInfo(name)
            info.size, info.mode = len(data), 0o600
            archive.addfile(info, io.BytesIO(data))
    return bundle.getvalue()


def provision(pi, upload, root, ssid, directory, source=None, rotate=False):
    candidate = prepare(directory, source, rotate)
    data = payload(ssid, read_key(candidate))
    upload(pi, INSTALL, data, root, pi.config["wifi_interface"],
           f"/var/lib/iwd/ap/{ssid}.ap", hashlib.sha256(data).hexdigest())
    current = candidate.parent / "device-key"
    if candidate != current:
        os.replace(candidate, current)
    return current
