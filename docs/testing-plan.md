# Local and Pi testing plan

Real Wi-Fi tests run on a dedicated Pi 4 with a custom Buildroot development
image owned by `BuildrootImages`. Ethernet SSH provides management, updates,
commands, and logs while iwd controls Wi-Fi. The SSH workflow belongs in this
repository and uses standard protocols; it has no SBC-controller or Pico dependency.
Ethernet is the primary development transport. Wi-Fi is the interface under test,
and optional Wi-Fi reachability checks do not change the management connection.
Only a working Wi-Fi station connection suppresses the provisioning AP; Ethernet
connectivity does not. For the current fresh-credential phone/AP test, the user
requested removal of the Pi's saved `Waller Creek Assembly Plant` profile;
that scoped removal is complete. Keep Ethernet management. Do not reconnect to the router
before the phone test is complete unless the user requests it.

**Current status (2026-10-12):** the `polis-rpi4-dev` image is built and installed.
Ethernet mDNS resolution and key-based SSH succeeded; `polis-dev-check` passed
on the Pi 4B, verifying storage, D-Bus, iwd, and Wi-Fi discovery. The initial
record and reusable SSH configuration are in ignored `.local/pi/` files.
The [parameterized update suite](../dev/pi/README.md) passed real-Pi checks for
two AArch64 builds, execution/log retrieval, incomplete-upload rejection,
rollback, and reboot/reconnect with persistent state and trusted SSH identity.
Evidence is in `.local/pi/update-acceptance.log`. Wi-Fi connected and obtained
an address; ping and SSH over Wi-Fi succeeded. Scoped credential removal passed
after forgetting, iwd restart, and Pi reboot: no saved test profile, no known
test network, no Wi-Fi reconnection, and Ethernet access restored. Cleanup
evidence is in `.local/pi/wifi-cleanup-acceptance.log`.
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
| `.local/` | Actual target settings, protected credential inputs, SDK/output paths, captured results | Ignored local data |
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

Steps 1–3 now have twenty-seven behavioral tests and one compiling doctest. Run
`cargo test --workspace --locked --offline`; the only external test executable
is `dbus-daemon`. The public contract fixture starts a private bus on a temporary
Unix socket, injects both connections explicitly, bounds the test to 10 seconds,
and kills/reaps the bus on teardown. No root or host session/system bus is used.
A sandbox must permit that local socket; inability to start the bus is a failure,
never a skipped test or a fallback to the host bus.

The Wi-Fi script rejects unexpected calls and uses paused Tokio time plus a
temporary directory for non-secret mock metadata. It exercises success,
authentication failure, timeout, explicit reconciliation, status publication,
and redacted Debug output. These are foundation tests, not evidence that the
future actor cancels real iwd actions or preserves real credentials.

Compile the independent client with
`cargo check --manifest-path tests/api-consumer/Cargo.toml --locked --offline --target-dir .target/api-consumer`.
It has a separate workspace and lockfile so daemon features cannot mask missing
API dependencies. Do not run it against the host bus for local acceptance.
Check package contents with
`cargo package -p wifi-setupd-zbus-api --list --allow-dirty --offline`; its explicit
allowlist includes only public source, a consumer example, documentation, and
Cargo metadata. Normal API dependencies exclude the daemon and development suite.

Add deterministic tests with each subsequent component. Test the public
service/client contract separately from the adapter's fake-iwd contract.
Step 2's private-bus fake covers scans, auth callbacks, AP/station transitions,
replacement/deletion scope, iwd/device replacement, and cancellation including
late mode-change completion. Step 3 adds paused-time actor tests for startup
grace, auto-connect races, a persistent AP, cancellation/cleanup retries, stale
results, timeout recovery, and confirmed authentication rejection. The production
exporter is tested on a private bus for completion ordering and client/service
reconnection, including a full actor → real adapter → fake iwd connection.
The UI later uses a simulated HTTP backend.

The isolated [iwd probe](../dev/iwd-probe/README.md) builds and runs on the Pi.
Hardware discovery/scanning, station association with Wi-Fi IPv4, AP startup
using an iwd `DisableHT=true` profile, explicit AP stop, and recovery across iwd
restart pass. AP startup returns while iwd keeps the AP running; there is no
automatic shutdown timer. Phone association/DHCP passed: the user reported
`192.168.16.2`, with the Pi independently observed at `192.168.16.1/28` and one
associated client. AP-to-station discovery/reconciliation timing, security-mode
coverage, and explicit profile
replacement remain pending. Normal Wi-Fi is valid for connection acceptance;
leave a successful user profile saved in iwd. Use a disposable profile or explicit
user direction before destructive replacement/deletion tests.

| Scenario group | Local evidence | Pi evidence |
| --- | --- | --- |
| First boot and success | No credentials → AP → connect → Idle; daemon remains callable | Phone joins AP, obtains DHCP, submits credentials, Pi joins target Wi-Fi without Internet |
| Failure and retry | Wrong password, unavailable network, timeout, repeated commands, stale results | Failure restores usable AP; retry succeeds; transient loss avoids switching loops |
| Rejected saved credentials | Visible saved network plus confirmed authentication rejection → scoped iwd Forget → provisioning; preserve unrelated profiles and do not delete for timeout/DHCP/backend failures | Change the dedicated test AP password, confirm rejection of the old saved credentials, verify their removal and successful onboarding with the new password |
| Interruptions | Cancellation, iwd object replacement, bus/service restart | Restart processes and reboot over SSH; manual power interruption separately verifies power-loss recovery |
| Persistence and API | Explicit same-SSID replacement discards old credentials even if the new attempt fails; unrelated profiles survive; clients resynchronize | iwd persists successful replacement across reboot; failed replacement returns to provisioning without restoring old credentials; no secrets in logs, properties, or signals |
| Portal and release | HTTP validation, UI reconnect, offline assets, dependency/package checks | Mobile captive behavior, direct-IP fallback, BusyBox startup, read-only root, development tools absent from release image |

Step 4 also verifies that HTTP stays running across AP/station transitions:
disconnected users see onboarding; connected users can reach the UI at the
Wi-Fi LAN address and list/delete selected saved networks. Confirm deletion
preserves unrelated profiles, requires the shared device/AP password, and that
mutations still validate Host/Origin and CSRF tokens. A brief address/radio transition is not a server shutdown.

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
independent API consumer to demonstrate that importing `wifi-setupd-zbus-api` does
not require the development suite.

## Step 4 portal acceptance

The [portal workflow](portal.md) builds actual Dioxus WASM assets and deploys the
runtime over verified Ethernet. Local mobile Chromium acceptance uses the real
HTTP server and actor with an in-memory adapter: authentication failure/retry,
lost-response guidance, connection success, CSRF validation and scoped deletion
pass. All assets load locally; passwords stay out of URLs and browser storage.

Next hardware check: forget/rejoin `wifi-setupd-test` on iPhone/iPad, confirm its
Wi-Fi sign-in sheet opens automatically without entering an IP address, submit
router credentials through the page, then verify the portal at the station LAN
address. Also exercise an incorrect password and AP restoration/retry. The
provision helper installs a per-device password for AP access and management;
startup reuses it. It never reconnects to the router on its own,
and the AP has no shutdown timer. Keep successful router credentials in iwd for
connected-management testing; scoped deletion remains an explicit user action.

Captive discovery checks cover UDP/TCP DNS, zero-TTL A records, empty AAAA/HTTPS
answers, malformed queries, automatic DNS shutdown on leaving onboarding,
Apple/Android/Windows HTTP redirects, and unchanged API/Origin restrictions.
The iwd AP fixture advertises its own DNS address through DHCP. DNS and probe
redirects are AP-only; Ethernet is never selected and station-LAN management
keeps normal Host/Origin checks. Actual iOS presentation still requires a phone.

**Live Pi evidence (2026-10-13):** the current ARM daemon and UI are deployed;
`dev/pi/captive_check.py` passes against the Pi: UDP/TCP DNS answers, Apple's
HTTP probe redirects, canonical portal HTML and rejected foreign-host API calls.
The test checks that the AP address is assigned to wlan0 before probing, so a
DNS server reachable through Ethernet cannot be mistaken for the Pi's AP.
The AP remains running for the iPhone check; automatic phone presentation is
not yet confirmed.

Runtime bring-up exposed missing D-Bus name-ownership policy in the existing
EROFS image. The development start helper adds a root-only policy through a
reversible `/run` bind mount and ReloadConfig; production integration belongs in
BuildrootImages. iwd also exited during the first station/AP transition; the
cause is not established. After restarting iwd and explicitly starting its AP
profile, the daemon adopted the AP and the captive checks passed. Cold-start
and iwd-exit recovery still require qualification; this is not a claim that
those failure paths passed on hardware.

The device password was subsequently rotated to three lowercase words followed
by three digits. All 19 development-helper tests pass. Restarting after rotation
reproduced the iwd exit; restarting iwd and explicitly starting the updated AP
profile restored service. The automatic station/AP transition remains unqualified.

The first iPhone attempt did not open the captive sheet and Safari used cellular.
The AP profile omitted `Gateway`; inspection of the installed iwd 3.12/ELL 0.83
source showed that DHCP omits the router option in this case, despite the man
page's claimed fallback. Provisioning now explicitly sets `Gateway=192.168.16.1`
alongside `DNSList`, and the existing Pi profile was updated without rotating its
password. The smoke test now checks these profile settings as well as DNS/HTTP.
This is not an on-wire DHCP check: phone rejoin and confirmation of its assigned
router and automatic sign-in sheet remain necessary.

The user subsequently confirmed successful onboarding to the router and portal
access through the station Wi-Fi IP. The hostname-capable daemon was then deployed
over verified Ethernet with `--http-hostname polis-dev.local`. The current
BuildrootImages Avahi diff was applied via a temporary bind mount, enabling IPv4
mDNS on eth0/wlan0 and disabling IPv6 transport/AAAA publication. Wi-Fi-scoped
mDNS returned the station address, hostname and IP HTTP requests returned 200,
foreign Host/Origin requests returned 403, and status remained Idle/Ready.
Ethernet SSH remained available using its pinned IPv4 and existing trusted key.
Evidence: `.local/pi/mdns-live-check.log`. The Avahi override is removed by reboot;
`dev/pi/mdns.py` reapplies the image-owned configuration without reflashing.
The multihomed laptop's default resolver can still select Ethernet; a phone on
the station Wi-Fi or an interface-scoped query tests Wi-Fi discovery directly.

Runtime hostname detection was subsequently deployed with no `--http-hostname`
argument or development `http_hostnames` override. The Pi's system hostname
`polis-dev` enabled `polis-dev.local` for portal HTML/API (200); foreign-host API
access remained forbidden (403). Evidence: `.local/pi/runtime-hostname-check.log`.
During this update the user confirmed deleting the router profile through the
portal. It remains absent and the setup AP was restored for further onboarding.
The existing iwd exit during automatic AP transition recurred and required an
iwd restart plus explicit AP start; runtime hostname verification does not
qualify that unresolved transition failure.

Active-network deletion now immediately shows reconnect instructions in the
portal. Mobile Chrome acceptance covers a held deletion request, stale Ready
polls, definite rejection/rollback, a lost response after deletion, and recovery
to onboarding. Actor tests cover immediate fallback without startup grace,
preserving an unrelated connection, and restarting an inactive AP interface.
Workspace tests (46 plus one doctest), formatting, Clippy and the actual WASM
build pass. The updated daemon/UI are deployed over verified Ethernet.

The iwd failure is now confirmed as SIGSEGV (exit 139). A user-authorized private
core inspection shows `util_address_to_string` called from
`network_bss_property_get_address` during ObjectManager `GetManagedObjects`.
Installed iwd 3.12/ELL 0.83 source shows network removal unregisters the parent
object without removing its BSS children before station BSS storage is freed.
A candidate cleanup patch is staged privately for validation; it is not yet a
qualified image fix. Restarting iwd lets the updated daemon restore the inactive
AP automatically. Router credentials remain absent as requested.

With explicit user approval, iwd alone was subsequently rebuilt and temporarily
installed over verified Ethernet. The fix is preserved in BuildrootImages at
`patches/musl/iwd/0001-network-unregister-bss-objects-on-removal.patch`.
Automatic station-to-AP fallback and three further daemon-driven station-scan/AP
cycles passed. Repeated full ObjectManager enumeration did not crash; the same
iwd PID survived all cycles, and no stale BasicServiceSet objects remained in
AP mode. Captive UDP/TCP DNS, OS probe redirects and portal HTTP checks also
passed afterward. Saved station profiles remained absent. The setup AP is left running.
Evidence: `.local/pi/iwd-fix-acceptance.log`.

The replacement is a reversible bind mount and disappears on reboot; the
original binary is retained. The image-owned patch uses the existing global
package-patch directory. Only the iwd package was rebuilt, not an SD image,
kernel or toolchain. Build/rollback details are in BuildrootImages
`docs/iwd-ap-recovery.md`. An actual phone reconnect followed by deleting its
active router profile remains the end-to-end acceptance check.

Captive redirects now prefer the daemon's runtime hostname (or first explicit
hostname override), retaining the listener IP only when no hostname is configured.
The ARM update is deployed. Live OS probes redirect to `http://polis-dev.local/`,
hostname HTML/API and captive DNS checks pass, and Avahi's wlan0 service record
resolves `polis-dev.local` to `192.168.16.1`. Eight HTTP tests and 20 development
helper tests pass, along with formatting and Clippy. The iPhone test must stay
inside its sign-in window: the user reports that leaving that window disconnects
the phone, so a separate Safari test is not required. Hostname resolution inside
the phone's captive window remains to be confirmed by rejoining the AP.
