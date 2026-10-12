# Local and Pi testing plan

Real Wi-Fi tests run on a dedicated Pi 4 with a custom Buildroot development
image owned by `BuildrootImages`. Ethernet SSH provides management, updates,
commands, and logs while iwd controls Wi-Fi. The SSH workflow belongs in this
repository and uses standard protocols; it has no SBC-controller or Pico dependency.
Ethernet is the primary development transport. Wi-Fi is the interface under test,
and optional Wi-Fi reachability checks do not change the management connection.

**Current status (2026-10-12):** the `polis-rpi4-dev` image is built and installed.
Ethernet mDNS resolution and key-based SSH succeeded; `polis-dev-check` passed
on the Pi 4B, verifying storage, D-Bus, iwd, and Wi-Fi discovery. The initial
record and reusable SSH configuration are in ignored `local/pi/` files.
The [parameterized update suite](../dev/pi/README.md) passed real-Pi checks for
two AArch64 builds, execution/log retrieval, incomplete-upload rejection,
rollback, and reboot/reconnect with persistent state and trusted SSH identity.
Evidence is in `local/pi/update-acceptance.log`. Wi-Fi connected and obtained
an address; ping and SSH over Wi-Fi succeeded. Scoped credential removal passed
after forgetting, iwd restart, and Pi reboot: no saved test profile, no known
test network, no Wi-Fi reconnection, and Ethernet access restored. Cleanup
evidence is in `local/pi/wifi-cleanup-acceptance.log`.
[Steps 0.1–0.3](implementation-plan.md) are complete.
The daemon will use zbus directly, without iwdrs.

## Existing environment

Inspection on 2026-10-12 found Ubuntu 26.04.1, Rust 1.99.0, and NetworkManager
1.54.3 on the laptop. `wlo1` was connected; Ethernet device `enx98fc84ec41a4`
was unavailable. NetworkManager continues to manage the laptop; real iwd
operations target the Pi.

The sibling `BuildrootImages` repository provides a Pi 4 musl/EROFS base with
persistent `/data`, plus optional iwd and firmware groups. Its speed profile
disables Ethernet and omits SSH/getty and `iwctl`. Our development image needs
explicit selections for these capabilities.

## Step 0 repository layout

The development suite and local configuration now exist at these paths.

| Path | Responsibility | Distribution |
| --- | --- | --- |
| `../BuildrootImages/` | Development image profile/overlays, build entry point, pinned build inputs | BuildrootImages development image tooling only |
| `dev/pi/` | SSH configuration example, remote commands, updates, logs, readiness checks | Repository development tooling only |
| `local/` | Actual target settings, protected credential inputs, SDK/output paths, captured results | Ignored local data |
| `crates/` | Daemon, public API, and web UI | Product build/package inputs |

Any Rust development helper gets a separate workspace under `dev/`. Product
crates never depend on that workspace or execute its scripts during builds.
Separate image composition excludes development services from production.

## 0.1 Image bring-up

Own the wifi-setupd development image profile, overlays, build entry point,
and pinned inputs in `BuildrootImages`, using its existing profile/group system.
This repository references a parameterized checkout and profile rather than
duplicating image definitions. Record the BuildrootImages revision,
selected groups, SDK identity, image checksum, and architecture with results.
Preserve musl, read-only EROFS, and writable `/data` so tests resemble deployment.

| Image concern | Required development selection |
| --- | --- |
| Management | Ethernet driver, DHCP or static address, key-based SSH and compatible file transfer |
| Wireless | iwd, `iwctl`, firmware, system D-Bus, DHCP and existing openresolv integration |
| Persistence | Persistent iwd storage under `/var/lib` backed by `/data`; private upload/state directories |
| Iteration | Execute uploaded binaries from writable storage, manage their process, collect logs, restore the previous artifact |
| Distribution | SSH access configuration, diagnostic clients, and test scripts are development-image-only inputs |

The existing iwd settings enable `EnableNetworkConfiguration=true` and
`NameResolvingService=resolvconf`; start there. Use the image builder's
`--development` option for routine iterations to omit automatic EEPROM setup
and data expansion. That option alone does not add SSH or required packages.

Install the initial image on the intended SD card as a separate explicit
operation. Retain a reproducible image for recovery. Routine updates over SSH
must not reflash storage or reset persistent iwd state.

Acceptance: boot, Ethernet SSH login, verified mounts and Wi-Fi discovery,
and successful reboot/reconnect. No custom diagnostic protocol is required.

## 0.2 Ethernet and updates

**Through a switch:** use DHCP when the switch is attached to a DHCP-serving
LAN. A standalone switch needs static addresses or a separate DHCP server.

**Direct cable:** choose a subnet that does not overlap the laptop's LAN, VPN,
or future provisioning AP. `192.168.77.1/24` on the laptop and
`192.168.77.2/24` on the Pi are examples, not assumed assignments. Configure the
laptop Ethernet profile without DNS or an IPv4/IPv6 default route. Pi Ethernet
uses the corresponding static address. No bridge or Internet sharing is needed.

Keep these suite parameters in ignored local configuration:

| Group | Parameters |
| --- | --- |
| SSH target | Host, port, user, identity file, known-hosts file, expected Pi identity |
| Interfaces | Pi Ethernet and Wi-Fi names, management address and addressing mode |
| Build | BuildrootImages checkout, SDK/linker, target triple, artifact paths |
| Remote state | Upload staging, active/previous artifact, daemon state and logs |

Use `aarch64-unknown-linux-musl` with the matching SDK linker. Cross-build the
current scaffold first. Upload to a temporary location, verify checksum and
compatibility, then activate it. Retain the previous artifact for explicit
rollback. Use writable storage for deployment; release-image installation
paths are a separate concern.

Bound command/reconnect waits, preserve exit statuses, and report the last
confirmed stage after connection loss. Query the target before replaying an
uncertain update. Use file transfer supported by the selected SSH server,
without an extra control daemon.

Acceptance: remote execution, output retrieval, update, rollback, interrupted
upload, and reboot/reconnect are repeatable from documented repository commands.

## 0.3 Wi-Fi readiness and cleanup

1. Check Ethernet SSH, iwd readiness, Wi-Fi interface identity, and absence of
   preexisting credentials for the selected dedicated test network.
2. Scan and connect using on-Pi `iwctl` over SSH. Supply credentials through
   protected input; keep them out of command arguments, shell history, and logs.
3. Verify iwd association and a usable Wi-Fi address while Ethernet SSH stays
   available. An Ethernet route cannot satisfy this check; Internet is unnecessary.
4. Disconnect and forget that network through iwd; verify its saved profile is
   gone and remove temporary secret inputs.
5. Restart iwd and reboot; verify Ethernet returns, the network remains unknown,
   and Wi-Fi does not autoconnect to it.

Use a test network whose credentials can be discarded. Scope deletion to that
network, preserving unrelated profiles. Failed connection attempts still trigger
cleanup. If SSH drops before cleanup, report it as pending and verify after
reconnection. Failed cleanup is a failed readiness check. Do not record secret
contents as evidence of removal.

Keep redacted results with image/iwd versions, Wi-Fi state/address, cleanup
outcome, and post-reboot state. This proves functional removal from iwd, not
forensic flash erasure. The `iwctl` readiness check is independent of the future
zbus adapter. Application work begins only after this sequence passes.

## Tests after Step 0

The scaffold currently has zero behavioral tests. Add deterministic tests with
each application component: simulated Wi-Fi that cannot fall back to real iwd,
temporary storage, controllable time, and a private D-Bus. Inject the private
bus connection explicitly; a session address does not redirect system-bus clients.
Test the public service/client contract separately from the adapter's fake-iwd
contract. The UI later uses a simulated HTTP backend.

| Scenario group | Local evidence | Pi evidence |
| --- | --- | --- |
| First boot and success | No credentials → AP → connect → Idle; daemon remains callable | Phone joins AP, obtains DHCP, submits credentials, Pi joins target Wi-Fi without Internet |
| Failure and retry | Wrong password, unavailable network, timeout, repeated commands, stale results | Failure restores usable AP; retry succeeds; transient loss avoids switching loops |
| Interruptions | Cancellation, iwd object replacement, bus/service restart | Restart processes and reboot over SSH; manual power interruption separately verifies power-loss recovery |
| Persistence and API | Prior credentials survive failed replacement including same SSID; clients resynchronize | Credentials survive reboot/interrupted updates; no secrets in logs, properties, or signals |
| Portal and release | HTTP validation, UI reconnect, offline assets, dependency/package checks | Mobile captive behavior, direct-IP fallback, BusyBox startup, read-only root, development tools absent from release image |

Mocks cannot establish driver support or mobile captive discovery. Verify Wi-Fi
success on its interface; a successful unbound ping may use Ethernet instead.
Use a phone as the provisioning client and a controlled router or separate
hotspot as the target. Preserve `/data` across persistence scenarios: reflashing
resets the state under test. A clean SSH reboot does not prove sudden power-loss
recovery. Manual power interruption needs no controller dependency in this repo.

## Release acceptance

Normal product builds, such as `cargo build --release --locked`, must never
invoke deployment commands or build development helpers. Any later release task
preserves that boundary. Add explicit package and image install allowlists.

Inspect product dependency graphs, crate package file lists, and release-image
contents. Verify that Step 0 scripts, probes, deployment settings, SSH access
configuration, credentials, and development services are absent. Compile an
independent API consumer to demonstrate that importing `wifi-setupd-api` does
not require the development suite.
