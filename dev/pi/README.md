# Pi development over Ethernet

Run commands from the wifi-setupd repository root. Python 3.11+, OpenSSH, and
the matching AArch64 musl Rust target/Buildroot linker are required. Image
definitions remain in BuildrootImages. This directory is outside Cargo's product
workspace and is never installed on the appliance.

Ethernet is the primary development transport for deployment, control, and logs.
Every remote command verifies the Pi serial, AArch64 architecture, and that its
SSH connection terminates on the configured Ethernet interface. Wi-Fi is only
the interface under test; connecting it does not change the management target.

## Configuration

Copy `dev/pi/config.example.toml` to ignored `.local/pi/config.toml` and fill in
the verified serial, paths, and interfaces. The current local setup is already
configured. Relative paths resolve from the repository root. Preserve the
compiler symlink name: Buildroot's wrapper depends on it.

The identity key and known-hosts file are referenced, never copied into the
suite. Unknown or changed SSH host keys fail; establish and verify a new target's
host key explicitly. `--config PATH` selects another configuration. The configured
application path must agree with the image's development init script.

When Avahi advertises on both Ethernet and Wi-Fi, set `host` to the verified
Ethernet IPv4 address so deployment cannot select Wi-Fi through mDNS. Preserve
the already verified SSH host key for that address. Refresh the target if its
Ethernet DHCP lease changes. The daemon derives its default portal name from
the Pi's runtime hostname. Optional `http_hostnames = ["custom.local"]` replaces
that default when `portal.py start` runs.

## Apply development mDNS settings without reflashing

Run `python3 dev/pi/mdns.py` over verified Ethernet. It reads the exact Avahi
configuration from the configured BuildrootImages checkout's development-access
overlay, verifies the uploaded checksum, bind-mounts it over the read-only
configuration, and restarts Avahi. A failed restart restores the previous config.
The override lasts until reboot; rerun this command after reboot while testing
an older image. Permanent image configuration remains in BuildrootImages.

Hostname HTTP access requires the updated daemon; no explicit hostname setting
is needed for the normal system name. From the station Wi-Fi LAN, open `http://polis-dev.local/`.
A laptop connected to both development Ethernet and Wi-Fi may resolve the shared
name to Ethernet; the portal intentionally listens only on Wi-Fi.

## Per-device AP and management password

Production integration belongs in BuildrootImages; follow the
[device provisioning handoff](../../docs/device-provisioning.md). For the
development portal, `python3 dev/pi/portal.py provision` generates or imports a
shared AP/management password independently of build/deploy. Its private local
record is `.local/devices/<Pi-serial>/device-key`. See the handoff for rotation,
recovery and the production startup contract.

## Build and update

```sh
python3 dev/pi/pi.py status
python3 dev/pi/pi.py build
python3 dev/pi/pi.py deploy .target/aarch64-unknown-linux-musl/release/wifi-setupd
python3 dev/pi/pi.py run
```

`build` compiles only the product daemon and its dependencies. Install the target
once with `rustup target add --toolchain 1.99.0 aarch64-unknown-linux-musl`.
`deploy` verifies the ELF target, streams it through SSH to a temporary file,
checks length and SHA-256, stops the existing app, preserves the previous binary,
and atomically activates the upload. Deployment does not execute the new artifact.

Use `run` for a foreground smoke check. The scaffold prints a message and exits;
this does not claim that a daemon is running. Use `start` for a long-running app
through the image's init script, `logs` for its last 100 log lines, and `stop` to
stop it. `rollback` restores the previous binary without starting it. Upload,
activation, rollback, and service control are serialized with a remote lock.

An incomplete or checksum-invalid upload cannot replace the active binary.
If a connection fails after activation, query `status` before retrying: ending
the SSH wait does not undo an accepted remote operation. `status` reports hashes
of active and previous artifacts. Rollback restores code only; it does not undo
future application changes to persistent data.

## Commands and reboot

Run `cargo make ssh` for an interactive shell using `.local/pi/config.toml`.
The session verifies the Pi identity and Ethernet connection and stays open until
you run `exit` or disconnect. For another configuration, use
`python3 dev/pi/pi.py --config PATH ssh`.

```sh
python3 dev/pi/pi.py exec uname -a
python3 dev/pi/pi.py check > .local/pi/readiness.log
python3 dev/pi/pi.py reboot
```

`exec` quotes each argument for SSH. For shell syntax, explicitly pass `sh -c`
and one quoted script. Do not pass Wi-Fi passwords in command arguments.

`reboot` creates a unique persistence marker, records boot and D-Bus machine
identities, sends exactly one reboot request, and waits up to the configured
deadline for a new boot ID. It checks the serial, Ethernet transport, persistent
marker, machine ID, and the already trusted SSH host key after reconnecting.
Failure or an ambiguous reply never causes an automatic second reboot.

## Supply Wi-Fi credentials

Run this yourself in an interactive terminal:

```sh
python3 dev/pi/pi.py wifi-connect
```

Enter the SSID, then the password at iwd's hidden prompt. The password travels
through the encrypted Ethernet SSH session; this suite does not store it in a
local file, log it, or place it in process arguments. iwd saves the resulting
network profile on the Pi. The command is not an automatic cleanup test.

If the network is not discovered, run a scan with
`python3 dev/pi/pi.py exec iwctl station wlan0 scan`, wait for it to finish, and
retry. Use the configured Wi-Fi interface if it is not `wlan0`.

After connecting:

```sh
python3 dev/pi/pi.py wifi-status
python3 dev/pi/pi.py wifi-ping
```

`wifi-status` obtains iwd status and the Wi-Fi IPv4 address over Ethernet.
`wifi-ping` reads that address over Ethernet, then sends three pings through the
configured laptop Wi-Fi interface. It requires a network path between those
Wi-Fi clients; client isolation may prevent replies even after successful
association. It does not use `polis-dev.local`, which can resolve to either interface.
Development continues over Ethernet after this optional sanity check.

## Forget the test network

Keep Ethernet connected. Use `wifi-status` to confirm the connected network is
the test network before disconnecting. Forget only that SSID, never the entire
iwd state directory. For the current image's `wlan0` interface:

```sh
python3 dev/pi/pi.py exec iwctl station wlan0 disconnect
python3 dev/pi/pi.py exec iwctl known-networks "YOUR TEST SSID" forget
python3 dev/pi/pi.py exec /etc/init.d/S40iwd restart
python3 dev/pi/pi.py reboot
python3 dev/pi/pi.py wifi-status
```

Check `python3 dev/pi/pi.py exec iwctl known-networks list` after forgetting,
after the iwd restart, and after reboot: the selected network must be absent.
Verify its saved profile is absent without printing its contents. On an otherwise
unconfigured test Pi, Wi-Fi must remain disconnected with no global Wi-Fi address.
This checks functional credential removal; it does not erase flash cells forensically.

## Validation

```sh
python3 -m unittest discover -s dev/pi -p 'test_*.py'
```

Unit checks cover argument quoting, error propagation, compiler symlink handling,
ELF validation, and avoiding reboot replay. Hardware acceptance separately checks
two different builds, execution/log retrieval, incomplete-upload rejection,
rollback, and reboot persistence. Local evidence belongs under `.local/pi/`.

## Captive portal checks

After deploying/provisioning/starting the [portal](../../docs/portal.md), run:

```sh
python3 dev/pi/captive_check.py
```

This checks AP-address ownership, UDP/TCP DNS, OS-probe redirects, portal HTML
and continued API Host restrictions over verified Ethernet. It never submits
router credentials. Follow it with the real iPhone/iPad check: forget/rejoin the
setup AP and confirm the sign-in sheet opens without manually entering an IP.
The setup password is the per-device key in the ignored local inventory.
