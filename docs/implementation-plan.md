# Proposed implementation milestones

Workspace setup and **Steps 0.1–0.3 are complete**. The Pi boots the development
image; the Ethernet update suite passed hardware acceptance, including rollback
and reboot persistence. iwd joined Wi-Fi, obtained an address, and passed Wi-Fi
ping/SSH checks. The selected credentials were forgotten and stayed absent after
iwd restart and Pi reboot. **Step 1 is next:** public API and local test foundation.
Remaining milestones describe planned work and require acceptance before advancing.

Use zbus directly for iwd integration. iwdrs may be consulted for examples but
will not be a dependency. These decisions supersede the corresponding portions
of the archived original prompt in `requirements.md`.

## 0. Prepare the Pi and prove the development loop

Make Pi bring-up and updates a documented, parameterized workflow owned by this
repository, with image definitions and builds owned by `BuildrootImages`.
Use Ethernet, SSH, and file transfer; there is no dependency on the
Polis SBC controller, Pico firmware, or a custom control protocol. Keep all
SSH/deployment support tooling under `dev/`, separate from the three product crates.

### 0.1 Create the development image

Create a reproducible Pi 4 Buildroot image with Ethernet, key-based SSH, iwd,
the `iwctl` diagnostic client, system D-Bus, Wi-Fi firmware, and persistent iwd
storage. Keep its profile, overlays, build entry point, and pinned image inputs
in `BuildrootImages`, using the existing tooling and profile system. This repo
references that profile through a parameterized BuildrootImages checkout and
records its revision and toolchain; it does not own a duplicate image definition.

Keep the production-like musl/read-only-root/writable-data layout. Enable the
Ethernet driver and management configuration; do not inherit the speed profile's
`no-ethernet` restriction. Include startup/logging support for iterative uploads
on the writable partition. Development SSH access and diagnostics belong only
to this image. Do not bake Wi-Fi passwords or private SSH keys into it.

Acceptance: build and identify the image by checksum; boot the intended Pi;
connect over Ethernet SSH; verify architecture, persistent storage, D-Bus,
iwd, and Wi-Fi device discovery. Reboot and regain SSH access. Initial SD
installation is a separate explicit action from routine binary updates.

### 0.2 Establish parameterized updates over Ethernet

Implemented in [dev/pi](../dev/pi/README.md); hardware acceptance passed on
2026-10-12. Ethernet remains the primary transport for all development work.

Provide a repo-local suite under `dev/pi/` for checking the target, remote
commands, cross-building, uploading, starting/stopping the development process,
collecting logs, and rebooting with bounded reconnect waits. Commit a configuration
example; keep actual host addresses, key paths, and local settings in ignored
`local/` files. Support both direct Ethernet and a switch-connected Pi.

Parameterize SSH host/port/user/identity/known-hosts, Ethernet and Wi-Fi interface
names, SDK location, and remote staging/state paths. Verify the target identity
before updates. Upload to a staging location, verify checksums, then activate;
retain the previous artifact for rollback. Routine updates preserve iwd state.
Use compatible SSH file transfer without requiring a custom remote daemon.

Acceptance: cross-build and upload the current scaffold, execute it remotely,
retrieve its output, replace it with a second build, and restore the previous
artifact. Demonstrate an interrupted upload leaves the active artifact intact,
and reboot/reconnect reports success or a bounded failure. Test tooling errors
must not be reported as successful deployments.

### 0.3 Prove iwd connection and credential cleanup

Passed on 2026-10-12. After forgetting, after iwd restart, and after reboot,
the selected profile was absent, the network was unknown to iwd, and Wi-Fi
remained disconnected with no IPv4 address. Ethernet SSH returned with the
trusted host key and expected Pi identity. Local evidence is recorded in
`local/pi/wifi-cleanup-acceptance.log`; no credential contents were read or logged.

Use SSH to operate iwd on the Pi, initially through the development image's
`iwctl` client. This proves the environment independently of the future zbus
adapter. Select a dedicated test network, scan, connect, and verify authenticated
association and a usable address on the Wi-Fi interface. Ethernet SSH must remain
available throughout, and Ethernet connectivity cannot satisfy the Wi-Fi check.

Forget only the selected test network through iwd, disconnect it, and verify its
saved credentials are removed. Restart iwd and reboot the Pi; confirm the network
remains unknown, does not autoconnect, and Ethernet management returns. Supply
credentials through protected input rather than command arguments, saved shell
history, logs, or committed files. A failed connection still runs scoped cleanup;
cleanup failure remains a failed check with a concrete recovery action.

Acceptance: a repeatable connect/verify/forget/reboot check with redacted results
and no retained credentials for the test network. **Application implementation
starts only after Steps 0.1–0.3 pass on the real Pi.**

### Development and release boundary

`dev/` scripts, deployment configuration, probes, and hardware
test suites are never dependencies of `wifi-setupd` or `wifi-setupd-api`.
Development image inputs stay in `BuildrootImages` and outside product packages.
Keep any future Rust development helper in a separate Cargo workspace under
`dev/`; do not hide it behind a product feature flag. Product build scripts must
not invoke SSH, deploy code, or include development assets.

`cargo build --release` and the eventual release packaging task build only
product artifacts. The API crate's package contains only its public contract,
required metadata, and consumer documentation. Use explicit package/install
allowlists and check dependency graphs, package file lists, and release-image
contents as tooling is introduced. Importing the API must not pull in any of
the Step 0 support suite. A release task, if added, preserves this boundary.

## 1. Public contract and local test foundation

Define stable D-Bus representations for states and errors, typed zbus proxies,
and the internal Wi-Fi adapter boundary. Introduce a scripted mock, injectable
bus connections, temporary persistence, and a status snapshot. Resolve stable
crate versions against Rust 1.99.0 and lock them when first used.

Keep iwd paths/errors out of the public API. Distinguish provisioning state from
link readiness and backend availability; `Idle` alone must not imply connected.
Choose explicit semantics for repeated Start/Stop requests, cancellation, and
LastError before exposing them to clients.

Acceptance: independent API client compiles; a private-bus contract test reads
properties and observes updates; a simulated Wi-Fi test runs without host
network access. No dependency on UI, root privileges, or physical hardware.

## 2. Implement the direct zbus iwd adapter

Implement typed zbus proxies behind the daemon's internal adapter boundary for
device discovery and object lifecycle, scans, authentication agents, connection
errors, known networks, AP mode, DHCP status, and iwd restarts. Use the selected
iwd version's D-Bus contract as the authority; iwdrs is optional example material
only. Keep internal iwd bindings out of `wifi-setupd-api`.

Reuse the working image and Ethernet/SSH suite from Step 0 and
[the testing plan](testing-plan.md). Run a minimal adapter probe/client before
building the complete workflow. Investigate
AP-to-station transitions and saved-credential replacement on the actual iwd
version, including rollback when replacing credentials for the same SSID.

Acceptance: actual Pi evidence for station association and IP configuration,
AP availability and phone DHCP lease, and recovery after iwd restart.
Demonstrate supported WPA2/WPA3 combinations;
report hardware limitations rather than pretending they are adapter failures.

## 3. Provisioning actor and public service

Implement a single Kameo Provisioner with typed messages, operation IDs,
bounded deadlines, cancellation, and read-only watch snapshots. Long network
operations run outside its mailbox. D-Bus handlers message the actor and emit
standard property changes plus completion signals.

Determine initial state from iwd and persistent configuration. Treat dropping a
future as insufficient to undo an already accepted iwd action: reconcile actual
device state before starting a conflicting operation. Resubscribe and fetch a
fresh snapshot after iwd name-owner changes, object replacement, or bus reconnect.

Acceptance: first boot, successful connection, repeated requests, timeout,
Stop during a transition, stale result rejection, and client reconnection all
pass deterministic tests. Daemon stays running after successful provisioning.

## 4. Recovery and the portal

Implement AP/station sequencing, retry policy, and durable credential handling.
Prove preservation of previously working credentials before destructive profile
replacement. Define staging, commit, rollback, file permissions, and crash
recovery from evidence about iwd's storage behavior; avoid a second competing
credential database without a demonstrated need.

Add Axum endpoints, then the Dioxus 0.7 web client using the repository skill.
For a single radio, scan before starting the AP, label cached results, and
support manual SSID entry. Do not promise live AP-mode scans until measured.
Return submission acceptance before intentionally tearing down the AP; the UI
explains the phone's disconnection and does not interpret it as authentication
failure. Restore a usable portal with the error after failed attempts.

Start with direct local-IP navigation. Determine required DNS and OS probe
responses from phone testing; an HTTP page with DHCP alone does not establish
captive-portal discovery. Add a small local DNS responder only when needed.
Bind the provisioning surface to the intended AP interface/address, validate
requests, and avoid credential exposure in logs, URLs, or public status.

Acceptance: end-to-end onboarding and bad-password retry in the simulator and
on the Pi; success requires Wi-Fi association and usable Wi-Fi IP configuration,
without an Internet requirement. Ethernet management cannot satisfy this check.

## 5. Deployment and failure qualification

Package generated web assets into the daemon and integrate a Buildroot package
with BusyBox startup after persistent storage, system D-Bus, and iwd are ready.
Provide the system bus policy and document filesystem ownership, configuration,
startup/shutdown, musl/glibc constraints, and ARM build commands. Keep development
services excluded from production packaging from the outset; verify the release
and library boundaries defined in Step 0.

Acceptance: the testing plan's full matrix passes, including power cuts during
credential updates and transitions, iwd/bus restarts, UI reconnect, and offline
mobile use. SSH can exercise process restarts and clean reboots; actual power
loss requires a separately arranged manual power interruption. Do not equate
the two or make a fixture controller a requirement of the repo workflow.
Check real release binary size and resource usage before optimizing.
Tests are added with their components, not postponed until this final milestone.

## Next implementation slice

Begin Step 1 by defining public provisioning state/error representations and
Start/Stop semantics, then typed zbus proxies and private-bus contract tests.
Keep the internal Wi-Fi adapter boundary mockable and separate from the public
API. Use the established Ethernet suite for subsequent Pi iterations.
