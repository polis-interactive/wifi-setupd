# BuildrootImages integration handoff: per-device setup password

This is the agreed integration contract for Step 5. BuildrootImages owns the
production image integration and per-device provisioning step. wifi-setupd does
not generate device identities or passwords. No iwd patch or hostapd is required
for this design; the earlier proposed password-free AP is superseded.

## One secret, two uses

Supply one password when provisioning the SD card. The same password is used to
join its setup AP and to authorize saved-network deletion through the portal.
The browser labels it **Device password**, with a reminder that it is the setup
Wi-Fi password. Router passwords submitted during onboarding are separate;
iwd remains their only persistent store.

The supported format is 8–63 printable ASCII characters without spaces.
BuildrootImages accepts the supplied value through its build/provisioning input
(for example, an environment variable) and writes configuration files, never a
secret compiled into the executable. An explicit per-device value replaces any
initial default in both files; only that value is accepted. Password generation,
an inventory service, and one-time override tracking are not required.

## Required production integration

1. **Build one reusable image.** Include wifi-setupd, its web assets and stock
   iwd. Keep the read-only root filesystem and the existing persistent writable
   `/data` layout. The SD-card provisioner writes the password files into the
   card's writable persistent storage before first boot; it does not need to
   modify the read-only root filesystem.
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
4. **Start the daemon after provisioning and mounting persistent storage.** Pass
   `--device-key-file /data/wifi-setupd/device-key` and the matching `--ap-profile`
   SSID to wifi-setupd. The daemon loads the management secret at startup, while
   iwd reads its AP profile through `StartProfile`. Missing key configuration
   disables HTTP deletion; an explicitly supplied missing/invalid file fails
   startup. Complete and verify both file writes before declaring the card
   provisioned; retry interrupted provisioning before booting it. The daemon
   already retries system D-Bus connection and observes iwd availability; a
   small init script is sufficient, without an application runtime wrapper.
5. **Preserve the files across boots and image updates.** Normal updates retain
   `/data` and must not reinstall a default over the supplied password. A full
   reflash that erases `/data` provisions both files again. If an existing card
   is explicitly reprovisioned, replace both files while wifi-setupd and its AP
   are stopped. Preserve unrelated saved router profiles. The next startup
   loads the replacement password for both uses; restarting only the portal
   does not reload an already-running iwd AP.

Production startup, filesystem setup, packaging and SD-card provisioning belong
in **BuildrootImages**. Its provisioner writes both matching files. wifi-setupd
reads the device key; iwd reads the AP profile. wifi-setupd does not synchronize
these files, and this flow requires no daemon changes or new password-setting
command. Do not install this repository's `dev/` suite or make product builds
depend on it. Production image integration remains Step 5.

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

The existing development helper has a separate generation/rotation workflow
over verified Ethernet SSH; it is not a production integration requirement. It
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

Provision a card with an explicitly supplied password. Verify that a phone can
join its AP and delete a selected saved network using that same password. A
wrong management password must fail, while onboarding remains usable without a
second password prompt. If a default was replaced, it must fail for both uses.
Verify that reboot and an image update preserving `/data` retain the supplied
password and unrelated router profiles. An interrupted card write must not be
reported as successful provisioning; repeat provisioning and verify both files
before boot. Keep secrets out of logs, URLs, status responses and browser
storage; the protected credential files intentionally contain secret material.

Local Python tests cover generation, import, key/PSK correspondence, private file
permissions, retries/rotation and incomplete-upload recovery. HTTP and browser
tests cover management authentication. These do not establish SD-card
provisioning or image-update acceptance; those remain integration checks.
