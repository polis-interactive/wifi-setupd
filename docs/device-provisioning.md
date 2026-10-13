# BuildrootImages integration handoff: per-device setup password

This is the agreed integration contract for Step 5. BuildrootImages owns the
production image integration and per-device provisioning step. wifi-setupd does
not generate device identities or passwords. No iwd patch or hostapd is required
for this design; the earlier proposed password-free AP is superseded.

## One secret, two uses

Generate one random password per physical device. The same password is used to
join its setup AP and to authorize saved-network deletion through the portal.
The browser labels it **Device password**, with a reminder that it is the setup
Wi-Fi password. Router passwords submitted during onboarding are separate;
iwd remains their only persistent store.

Use a cryptographic random generator to select three readable lowercase words,
concatenated without separators, followed by exactly three random digits
(including leading zeros). For example, `cedarmapleriver007` illustrates the
format; never use this example as a default. The development helper uses this
format for new passwords and explicit rotations. Existing passwords remain
unchanged until rotation. The supported imported format is 8–63
printable ASCII characters without spaces. Never derive the password from the
serial number, MAC address, or a fleet-wide default. Serial numbers identify
records; they are not secrets. Retain the device/password association in the
provisioning station's private inventory for labeling, QR codes or later service.

## Required production integration

1. **Build one reusable image.** Include wifi-setupd, its web assets and stock
   iwd. Per-device passwords are injected after flashing or by an explicit
   provisioning stage, never compiled into the daemon or common image. Keep the
   read-only root filesystem and the existing persistent writable `/data` layout.
2. **Install the shared password on persistent storage.** Proposed production
   location: `/data/wifi-setupd/device-key`, root-owned mode `0600`, with a
   root-owned `0700` parent. Filesystem ownership may be adjusted for the actual
   service account while retaining restricted access. These production paths
   are a contract to implement, not already-installed files.
3. **Provision iwd's AP profile using the same value.** Install
   `/var/lib/iwd/ap/<setup-SSID>.ap` in iwd's persistent storage, mode `0600`.
   Include `[Security] Passphrase=<device password>`, or derive its
   `PreSharedKey` using WPA2 PBKDF2-HMAC-SHA1, 4096 iterations, 32 bytes, and the
   exact SSID as salt. The development helper uses the derived PSK. On the Pi,
   retain `[General] DisableHT=true` and explicit IPv4/DHCP configuration.
   Set both `[IPv4] Gateway` and `DNSList` to the AP's own IPv4 address for
   captive discovery. Explicitly set the gateway: the installed iwd 3.12/ELL
   0.83 omits DHCP's router option when it is absent, despite the documented
   fallback. A DNS server alone does not provide clients a default Wi-Fi route;
   wifi-setupd answers AP-only UDP/TCP DNS on port 53 and HTTP probes on port 80.
   Reserve those ports on the AP address and allow clients to reach them. A
   per-device SSID is useful for identification; changing it requires deriving
   the PSK again.
4. **Wire up startup after provisioning and iwd readiness.** Pass
   `--device-key-file /data/wifi-setupd/device-key` and the matching `--ap-profile`
   SSID to wifi-setupd. The daemon loads the management secret at startup, while
   iwd reads its AP profile through `StartProfile`. Missing key configuration
   disables HTTP deletion; an explicitly supplied missing/invalid file fails
   startup. Do not start with half-installed provisioning files.
5. **Make rotation a provisioning operation, not an image rebuild.** Stop
   wifi-setupd and its AP, stage/validate the new secret and AP profile, replace
   both, then restart. Use an incomplete-operation marker or equivalent recovery
   so power loss cannot start the portal with mismatched AP and management
   passwords. Retry interrupted updates with the same candidate secret. Existing
   saved router profiles must remain untouched. Clients must use the new setup
   Wi-Fi password after rotation; restarting only the HTTP service does not
   reload an already-running iwd AP.

Production startup, filesystem setup, packaging, inventory ownership and
provisioning hooks belong in **BuildrootImages**. Do not install this repository's
`dev/` suite or make product builds depend on it. Shared device-password support
is implemented in the runtime; production image integration remains Step 5.

## Hostname access integration

At startup, the daemon reads the system hostname with `gethostname()` and allows
`<first-label>.local` by default, matching Avahi's normal hostname derivation.
For example, both `polis-dev` and `polis-dev.local` yield `polis-dev.local`.
Buildroot only needs to configure the system hostname and Avahi; no application
hostname environment variable or build-time injection is required.

Repeatable `--http-hostname NAME` arguments replace the automatic default when
custom aliases are needed. The daemon does not run an mDNS responder. Keep the
advertised name stable and unique per device and account for Avahi conflict
renaming: an unconfigured renamed alias will be rejected. Restart the daemon
after changing the system hostname or override arguments. Invalid system names
fail startup with a validation error; explicit overrides bypass system lookup.

The current development image restricts Avahi to `allow-interfaces=eth0`.
Enable advertising on the intended Wi-Fi interface as well, using its current
IPv4 address across AP/station transitions. HTTP currently binds only IPv4 on
Wi-Fi; do not advertise an unusable IPv6 portal endpoint. Preserve verified
Ethernet SSH: if the shared hostname can resolve to Wi-Fi after this change,
give development SSH a deterministic Ethernet target in its ignored config.
Do not expose the portal on Ethernet merely to accommodate name resolution.

Acceptance: after onboarding, resolve the hostname from another device on that
Wi-Fi LAN, open its HTTP URL, and use saved-network management with the device
password. Confirm IP access still works, arbitrary Host/Origin values fail, and
the hostname resolves to the current address after reconnect/reboot. The daemon
side is covered by local HTTP tests; real mDNS discovery is image/hardware work.

## Development implementation and commands

The development helper provides this workflow over verified Ethernet SSH. It
uses `.local/devices/<configured-Pi-serial>/device-key` as the local inventory
record and `/data/wifi-setupd-portal/device-key` on the Pi. These are development
paths, separate from the proposed production path above.

```sh
# First provisioning, or reinstall the existing per-device secret:
python3 dev/pi/portal.py provision
python3 dev/pi/portal.py start
```

The portal must be stopped before provisioning. Existing development AP profiles
are explicitly replaced by `provision`; build/deploy/start never rotate secrets.
The helper returns the local record's path and never prints the password. Use
that protected file when joining the AP or supplying the portal's device password.

```sh
# Generate and install a replacement without rebuilding:
python3 dev/pi/portal.py stop
python3 dev/pi/portal.py provision --rotate-key
python3 dev/pi/portal.py start
```

To supply a password from an existing provisioning inventory instead of
randomly generating one, use `provision --key-file /path/to/private/device-key`.
The filename goes in process arguments; its contents travel only over SSH stdin.
Use `--config PATH` to select each target's verified serial and Ethernet details.

A failed or uncertain update retains `device-key.pending` locally. Retry
`provision` without either key option to reinstall that candidate and finish the
update. The previous local record stays intact until installation is confirmed.
On the Pi, a shared lock prevents concurrent start/provision operations; the
incomplete-operation marker blocks startup after a partial file replacement.
The helper switches the radio to station mode during provisioning so a later
AP start reads the new profile. It does not delete saved router credentials.

## Acceptance for the integration agent

Verify on hardware that two devices provisioned from the same image receive
different secrets; each phone can join with its own device password and delete a
selected saved network using that same password. A wrong management password
must fail, while onboarding remains usable without a second password prompt.
After rotation, verify the new password works for both purposes, the old one
fails for both, unrelated router profiles survive, and reboot preserves the
new secret. Exercise interruption between the two file updates and confirm
startup waits for recovery. Confirm no per-device secret is present in the
shared image, build outputs, logs, URLs, status responses or browser storage.

Local Python tests cover generation, import, key/PSK correspondence, private file
permissions, retries/rotation and incomplete-upload recovery. HTTP and browser
tests cover management authentication. These do not establish phone/Pi rotation
acceptance; that remains an integration check.
