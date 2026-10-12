# Proposed implementation milestones

Workspace setup and **Steps 0.1–0.3 are complete**. The Pi boots the development
image; the Ethernet update suite passed hardware acceptance, including rollback
and reboot persistence. iwd joined Wi-Fi, obtained an address, and passed Wi-Fi
ping/SSH checks. The selected credentials were forgotten and stayed absent after
iwd restart and Pi reboot. **Step 1 is complete:** public API and local test
foundation. **Step 2 is implemented and accepted by the user for advancement:**
the direct zbus adapter and phone/AP DHCP checks pass; remaining hardware
qualification is recorded below. **Step 3 is implemented:** the Kameo actor,
production D-Bus exporter, runtime, and deterministic acceptance tests pass.
Step 4 is next; its HTTP/UI work has not started.

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

Completed on 2026-10-12. The [public contract](../crates/wifi-setupd-api/README.md)
defines string wire values, a coherent status tuple, typed proxies/errors,
idempotent Start/Stop, cancellation reconciliation, and LastError retention.
The daemon has a Wi-Fi adapter trait and read-only watch snapshots. Its strict
scripted test adapter uses temporary metadata storage and paused Tokio time;
it has no path to a real backend. No credential database is introduced.

Acceptance passed: the independent `tests/api-consumer` workspace compiles;
the private-bus fixture reads properties, observes updates, checks typed
rejections/repeated commands, receives completion, and supports a fresh client.
Simulated success, failure, timeout/reconciliation, temporary storage cleanup,
and credential Debug redaction pass. Formatting, workspace build, and strict
Clippy pass. The API package allowlist excludes development scripts and tests;
its normal dependency graph contains no daemon, UI, iwd, or Kameo.
This validates the contract and test foundation; actual actor cancellation and
credential durability remain Steps 3–4. Production service startup is not added.

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

Implementation is in [the daemon's internal adapter](../crates/wifi-setupd/src/wifi/iwd/mod.rs),
with [design notes](iwd-adapter.md) and a separate
[development probe](../dev/iwd-probe/README.md). Private-bus tests cover actual
zbus method/property signatures, callbacks, replacement, cancellation, and
object/name-owner changes. In-flight mode changes and Forget replies survive
caller cancellation so a late reply cannot be mistaken for completed cleanup.

Pi evidence on 2026-10-12: the AArch64 musl probe discovers iwd 3.12, scans,
observes a successful user-entered station connection with Wi-Fi IPv4, and
recovers in the same adapter instance across iwd restart. A shutdown read
timeout exposed by the Pi now produces Unavailable/Unknown within three seconds;
subsequent observations recover. Non-secret evidence is in ignored
`local/pi/step2-station.log` and `local/pi/step2-iwd-restart.log`.
The working user network profile was initially retained in iwd. The user later
explicitly requested fresh-credential testing: the `Waller Creek Assembly Plant`
profile has now been forgotten through iwd and verified absent from its saved
network list. The Pi remains disconnected for the phone/AP test.

AP startup initially failed: the default iwd HT channel configuration was
rejected by the Pi's brcmfmac driver (`START_AP failed: -52`). An iwd AP profile
with `DisableHT=true` starts successfully on channel 6 with an AP-side IPv4
address. The adapter now supports `StartProfile` without reading/writing secrets.
The dev wrapper supplies a temporary iwd profile and removes it after startup;
the AP continues running in iwd after the probe/SSH command exits. At the user's
request there is **no automatic AP shutdown timer**: use `stop-ap` explicitly.
Persistent AP startup, scoped temporary-profile cleanup, and explicit return to
station mode pass on hardware. Evidence is in ignored
`local/pi/step2-ap-iwd-diagnostics.log`, `local/pi/step2-ap-address.log`, and
`local/pi/step2-ap-persistent.log`. Production AP image configuration belongs in
BuildrootImages; phone-side DHCP is not established by the Pi's own address.
The saved router profile reconnects to Ready afterward. An immediate connect
before station discovery populated returned NetworkUnavailable, with an Internal
cleanup error; retry after discovery succeeded. Qualify this transition race and
reconciliation before accepting the AP-to-station connection workflow.

Phone/AP DHCP passed on 2026-10-12: the user joined `wifi-setupd-test` and
reported phone IPv4 `192.168.16.2`. A simultaneous read-only Pi check confirmed
the AP was started, one station was associated, and wlan0 held
`192.168.16.1/28`, placing the phone on the AP subnet. The AP remains running;
no portal or credential-submission UI exists at this milestone.

Still pending: AP-to-station discovery/reconciliation timing; explicit
replacement/failure hardware checks; WPA2/WPA3-only/mixed qualification.
The available network's exact negotiated security has not been established.
IPv6-only readiness is not yet qualified. iwd 3.12's generic connection error
does not reliably identify rejected credentials; the automatic-removal policy
below requires stronger evidence than a generic Failed reply. The user explicitly
accepted the Step 2 implementation and authorized Step 3; these remain documented
hardware qualification gaps, not claims of completed coverage.

Credential policy clarified on 2026-10-12: iwd is the sole credential store.
Allow iwd to auto-connect using its saved profiles. If a visible saved network
rejects its stored credentials with a confirmed authentication failure,
wifi-setupd must forget that specific network through iwd and return to
provisioning so the user can supply working credentials. Do not retain a
credential known to be rejected. An absent SSID, generic connection failure,
timeout, DHCP failure, or backend interruption alone does not establish invalid
credentials and must not trigger deletion. Never forget unrelated profiles.
Explicit user-requested replacement also takes precedence: overwrite the saved
credentials for the selected SSID through iwd, even if the old credentials might
still work. Where iwd has no replacement method, Forget the matching profile
and connect with the submitted credentials; iwd persists the replacement on
its own terms. If the new credentials fail, return to provisioning without
restoring the deliberately discarded old credentials. Scope the target by SSID
and security type so unrelated profiles remain intact. Distinguish explicit
replacement from startup/retry operations, which must not silently erase profiles.
These requirements supersede credential-preservation/rollback language below
for confirmed-invalid credentials and user-requested replacement. Preserve
unrelated profiles. Do not introduce an application credential database or copy
iwd secrets into an application rollback store.

Implement typed zbus proxies behind the daemon's internal adapter boundary for
device discovery and object lifecycle, scans, authentication agents, connection
errors, known networks, AP mode, DHCP status, and iwd restarts. Use the selected
iwd version's D-Bus contract as the authority; iwdrs is optional example material
only. Keep internal iwd bindings out of `wifi-setupd-api`.

Expose saved-network metadata listing and scoped deletion now, for the connected
management UI in Step 4. Listing must not read or expose stored secrets.

Reuse the working image and Ethernet/SSH suite from Step 0 and
[the testing plan](testing-plan.md). Run a minimal adapter probe/client before
building the complete workflow. Investigate
AP-to-station transitions and explicit saved-credential replacement on the actual
iwd version, including failure after the selected old credentials are discarded.

Acceptance: actual Pi evidence for station association and IP configuration,
AP availability and phone DHCP lease, and recovery after iwd restart.
Demonstrate supported WPA2/WPA3 combinations;
report hardware limitations rather than pretending they are adapter failures.

## 3. Provisioning actor and public service

**Implemented and locally validated.** See [actor/service design and runtime
configuration](provisioner.md). The executable runs the production system-bus
service using an explicit interface and preconfigured iwd AP profile. The Pi's
running diagnostic AP was not changed or replaced during Step 3 implementation.

Twenty-seven behavioral tests plus one doctest pass, including the actor, real
adapter against fake iwd, and production D-Bus exporter in one private-bus test.
Formatting, strict Clippy, native compilation, the independent API consumer,
and AArch64 musl compilation pass. Hardware qualification of the new runtime,
production image AP configuration, and remaining iwd authentication/security
coverage remain explicit follow-up work; the HTTP portal is Step 4.

Implement a single Kameo Provisioner with typed messages, operation IDs,
bounded deadlines, cancellation, and read-only watch snapshots. Long network
operations run outside its mailbox. D-Bus handlers message the actor and emit
standard property changes plus completion signals.

Determine initial state from iwd and persistent configuration. Treat dropping a
future as insufficient to undo an already accepted iwd action: reconcile actual
device state before starting a conflicting operation. Resubscribe and fetch a
fresh snapshot after iwd name-owner changes, object replacement, or bus reconnect.

Let iwd attempt saved-network auto-connection during a bounded startup window
before opening the AP. Successful association and usable Wi-Fi IP configuration
produce Idle/Ready with the provisioning AP off. Only a working Wi-Fi station
connection suppresses automatic AP provisioning; Ethernet connectivity does not.
Ethernet remains available for management while testing the disconnected/AP path.
A confirmed rejection of stored credentials for a visible
network triggers scoped Forget through the adapter, then provisioning. If iwd's
automatic attempt exposes only a generic failure, obtain sufficient evidence
before classifying the credentials as invalid; visibility alone is not proof.

Acceptance: first boot, successful connection, repeated requests, timeout,
Stop during a transition, stale result rejection, and client reconnection all
pass deterministic tests. Daemon stays running after successful provisioning.

## 4. Recovery and the portal

Implement AP/station sequencing, retry policy, and durable credential handling.
Apply the removal and explicit replacement policies from Step 2. Do not restore
credentials that were rejected or deliberately overwritten. Preserve unrelated
profiles, and define crash recovery around iwd's own persistence behavior. iwd
is the sole credential store; no application credential database or secret
backup/rollback store is permitted.

Add Axum endpoints, then the Dioxus 0.7 web client using the repository skill.
Keep the HTTP server running whether Wi-Fi is connected or disconnected. In
disconnected/AP mode, focus the UI on seamless onboarding. In connected station
mode, serve the UI on the device's Wi-Fi LAN address and offer connection status
plus listing/deletion of old saved networks through iwd. Success changes the UI
mode rather than shutting down the server. A radio/address transition can
temporarily interrupt reachability; the server process itself stays available.
Define access control for saved-network mutations before exposing them on the
station LAN. Deletion is an explicit user action scoped to the selected profile.
For a single radio, scan before starting the AP, label cached results, and
support manual SSID entry. Do not promise live AP-mode scans until measured.
Return submission acceptance before intentionally tearing down the AP; the UI
explains the phone's disconnection and does not interpret it as authentication
failure. Restore a usable portal with the error after failed attempts.

Start with direct local-IP navigation. Determine required DNS and OS probe
responses from phone testing; an HTTP page with DHCP alone does not establish
captive-portal discovery. Add a small local DNS responder only when needed.
Serve on the intended Wi-Fi AP/station interface addresses as they change;
do not unintentionally expose management on every interface. Validate requests
and avoid credential exposure in logs, URLs, or public status.

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

Step 4: implement the HTTP onboarding and connected-network management workflows,
then the Dioxus portal. Use the actor's internal submission method with explicit
replacement intent. Carry forward AP-to-station hardware qualification, positive
authentication-failure classification, and WPA2/WPA3 coverage. Configure the
production iwd AP profile in BuildrootImages before deploying the runtime.
