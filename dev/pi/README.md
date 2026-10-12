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

Copy `dev/pi/config.example.toml` to ignored `local/pi/config.toml` and fill in
the verified serial, paths, and interfaces. The current local setup is already
configured. Relative paths resolve from the repository root. Preserve the
compiler symlink name: Buildroot's wrapper depends on it.

The identity key and known-hosts file are referenced, never copied into the
suite. Unknown or changed SSH host keys fail; establish and verify a new target's
host key explicitly. `--config PATH` selects another configuration. The configured
application path must agree with the image's development init script.

## Build and update

```sh
python3 dev/pi/pi.py status
python3 dev/pi/pi.py build
python3 dev/pi/pi.py deploy target/aarch64-unknown-linux-musl/release/wifi-setupd
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

```sh
python3 dev/pi/pi.py exec uname -a
python3 dev/pi/pi.py check > local/pi/readiness.log
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
association. It does not use `polis-dev.local`, which advertises Ethernet here.
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
rollback, and reboot persistence. Local evidence belongs under `local/pi/`.
