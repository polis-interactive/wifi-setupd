# Local Wi-Fi portal (Step 4)

The daemon serves a Dioxus 0.7.10 web app through Axum. It stays running across
AP/station transitions, binding port 80 only to IPv4 addresses on the configured
Wi-Fi interface. Ethernet addresses are not selected. While the setup AP is
awaiting credentials, the daemon serves captive DNS and redirects OS connectivity
probes to the setup page. Joining the AP should open the phone's Wi-Fi sign-in
sheet; the OS decides when to display it, so this requires phone qualification.

## Try it on the Pi

The development runtime has a separate binary/assets deployment, leaving image
startup integration for Step 5. Run from the repository root:

```sh
python3 dev/pi/portal.py build
python3 dev/pi/portal.py deploy
python3 dev/pi/portal.py provision
python3 dev/pi/portal.py start
```

`build` requires `dx` 0.7.10 and the `wasm32-unknown-unknown` Rust target; use
`--dx /path/to/dx` if it is not on PATH. Cross-linker/SSH settings come from
ignored `.local/pi/config.toml`. The current checkout also has an ignored
`.local/tools/` tool bundle, which the helper automatically uses for offline
rebuilds (dx 0.7.10, wasm-bindgen 0.2.129, esbuild 0.27.3 and Binaryen 120).
These are development tools, never product dependencies or shipped assets.

`provision` generates one password for this device and installs it for both AP
access and saved-network management. The local record is
`.local/devices/<Pi-serial>/device-key`, with private file permissions; its path
is printed, not its contents. The portal must be stopped before provisioning.
Use `provision --key-file PATH` to import an existing device password or
`provision --rotate-key` to generate a replacement. Restart afterward; no image
or binary rebuild is needed. A plain `provision` reuses the existing/pending key.

The development AP profile uses `DisableHT=true` and `192.168.16.1/28`.
BuildrootImages will own production per-device provisioning and startup wiring.
See the [integration handoff](device-provisioning.md) for the exact contract,
rotation commands and interrupted-update recovery.

Join `wifi-setupd-test` on the phone and wait for its Wi-Fi sign-in screen.
Enter the router SSID and password in that page. On iPhone/iPad, Settings →
Wi-Fi → the network's information button → **Join Network** also opens the
sign-in screen. When testing an AP saved before captive discovery was enabled,
forget/rejoin it so the phone obtains the updated DNS lease and reruns detection.
The numerical address remains a diagnostic fallback, not the normal entry flow. The AP has no lifetime/shutdown timer.
After success, join the router's Wi-Fi and open the Pi's new Wi-Fi IP address;
`python3 dev/pi/portal.py status` prints its current IPv4 address over Ethernet.
For hostname access, the daemon reads the system hostname at startup and allows
`<first-label>.local`. Configure Avahi to advertise that name on Wi-Fi
(BuildrootImages integration); `--http-hostname` is an optional override.
Then use `http://polis-dev.local/`, adding the port for non-default HTTP ports.
The original development image advertises only on Ethernet; the live Avahi
override enables Wi-Fi discovery until reboot. Hostname acceptance alone does
not make the name discoverable.
The page changes to connected-network management. If connection fails, rejoin
the setup AP and use its Wi-Fi sign-in screen to retry.

Other development commands:

```sh
python3 dev/pi/portal.py logs
python3 dev/pi/portal.py stop
```

For an update, build/deploy, explicitly stop the old runtime, then start the new
one. The current runtime keeps its existing release binary while an update is
uploaded. This helper does not install a boot service or change image definitions.
On the current read-only development image, `start` also copies the existing
`/etc/dbus-1` configuration into a temporary `/run` directory, adds a policy
allowing root to own/call only `tv.polis.WifiSetup1`, bind-mounts that directory
at `/etc/dbus-1`, and reloads D-Bus configuration. Existing policies are preserved;
D-Bus/iwd are not restarted. Reboot removes the temporary mount. Production
service policy and image integration remain owned by BuildrootImages.

## Connection and deletion behavior

The Networks field combines cached scan suggestions with typed SSID entry in
one editable combobox, using the [WAI-ARIA manual-selection pattern](https://www.w3.org/WAI/ARIA/apg/patterns/combobox/examples/combobox-autocomplete-list/). Adopting an already-running AP may leave this
list empty. It does not scan a single radio while the phone is using the AP.

The HTTP flow has two phases: `POST /api/connect` validates and reserves a
request, returning a random attempt ID without changing the radio. After
receiving acceptance, the browser clears its password field and calls
`POST /api/connect/{id}/begin`. Only then does the actor tear down the AP and
connect. Repeating begin for the last accepted attempt is idempotent. An unstarted
reservation expires after 30 seconds and drops its credentials; that timer
never stops the AP. Requests remain only in memory, and iwd persists successful
credentials. Submitting through the portal always replaces the same network's
saved profile, without a separate toggle or restoring its previous secret on failure.

A lost response means the outcome is unknown, not that the password was rejected.
The UI explains how to rejoin either network and polls again when reachable.
Backend authentication errors get specific wording only when the adapter has
positive evidence; generic iwd failures remain generic errors.
Interacting with the page dismisses the connection error. Identical status polls
keep it dismissed; a new failed connection attempt can show a fresh error.

`GET /api/status` exposes state, the observed `connected_ssid`, cached networks, saved-network metadata and a
CSRF token; no Wi-Fi secrets. `POST /api/forget` takes a selected SSID/security pair currently known to iwd.
Deletion requires the **Device password**, the same password used to join the
setup AP. The browser holds it only in memory. Deletion goes
through the actor and iwd, preserving unrelated profiles. The UI immediately
hides the selected network without a success notification. Rejected requests
restore the row and reopen the confirmation with the error; subsequent status
polls reconcile accepted requests. Deleting the current network disconnects
Wi-Fi and allows automatic AP fallback when no working Wi-Fi connection remains.

All mutations and API reads require an allowed local Host (IP or hostname),
and any Origin must match that request's Host and port. Mutations also require a per-listener CSRF token. During
AP onboarding only, foreign-host GET/HEAD requests outside `/api` receive a
fixed 302 redirect to the canonical setup page. These requests never serve the
UI/API at a foreign origin, reflect query parameters, or accept credentials. Bodies are capped at 4 KiB and
errors do not echo payloads. Responses use no-store and deny framing. HTTP is
available to clients that can reach the portal on the device’s Wi-Fi interface; transport encryption
and remote access are outside this slice. Station-LAN mDNS is supplied by Avahi.

## Local UI with fake Wi-Fi

The UI keeps polling, requests and form state in `src/state/`, reusable controls
in `src/components/` (including the saved-network list), setup/joining/connected
views in `src/pages/`, and the
shared header, notices and footer in `src/layout/`. `main.rs` only launches and
composes the app. Requests belong to the app scope so switching pages does not
cancel an accepted connection request. The Connected page owns its SSID heading
and saved-network management; the SSID comes from backend observations so it
also works on refresh and after iwd auto-connect.

From the repository root, run:

```sh
cargo make dev
```

Open **http://127.0.0.1:8080**. The task runs `dx serve` with the UI’s
`dev` feature and the local simulator. RSX and CSS edits hot reload; other Rust
changes trigger a rebuild and browser reload. It needs no Pi configuration, running daemon, iwd service,
or D-Bus connection, and never changes host Wi-Fi. Stop with Ctrl+C.

Edit [`dev/portal-sim/example.toml`](../dev/portal-sim/example.toml) to define
fake SSIDs and accepted passwords:

```toml
[[networks]]
ssid = "Home Wi-Fi"
password = "working-password"
connect_delay_ms = 2000

[[networks]]
ssid = "Workshop"
password = "workshop-password"
connect_delay_ms = 5000
```

Only an exact configured SSID/password pair succeeds. Wrong passwords produce
authentication errors, unknown/invisible SSIDs produce network-unavailable
errors, and an empty password defines open Wi-Fi. The example includes
`Home Wi-Fi` / `working-password` and `Workshop` / `workshop-password`.

Set `saved = true` on an entry to prepopulate saved networks, and `visible = false`
to omit it from discovery. Set top-level `initially_connected = "Home Wi-Fi"`
to start directly in the connected management view. Each network’s
`connect_delay_ms` controls its connection-attempt delay
(default 2000 ms, maximum 60000 ms), including rejected passwords.
`port`, `ap_name` and `device_key` remain top-level settings. The example device
password is `simulator-device-password`; it is only a local test fixture.

The simulator holds all changes in memory. Restart `cargo make dev` after editing
TOML or simulator Rust code; UI edits are watched automatically. Restarting resets
saved/deleted networks to the fixture. Use
fake credentials in these files. A private fixture can live in ignored `.local/`:

```sh
UI_MOCK_CONFIG=.local/portal/networks.toml cargo make dev
```

The fixture's `port` selects the browser-facing Dioxus server (default 8080).
The development API uses reserved loopback port 8089; Dioxus proxies `/api`
requests to it. Only the simulator adapts the matching local frontend Origin;
production HTTP validation is unchanged. Ctrl+C stops both development servers.
If a port is occupied, stop the previous preview before starting `dev`.

`cargo make ui` remains a static release preview without watching or hot reload.
Use it for testing the release assets served directly by the simulator.

The fixture and simulator live under `dev/` in a separate Rust workspace. The
UI uses its regular HTTP contract against a simulated radio through the local
HTTP server/actor; production UI assets contain neither fixtures nor a mock mode.

Prerequisites: `cargo-make`, Dioxus CLI 0.7.10, and the Rust
`wasm32-unknown-unknown` target. The current checkout's `.local/tools` bundle is
used automatically; `DX=/path/to/dx` overrides CLI selection. `cargo make ui-build`
only builds the web assets. No Pi cross-compiler is involved.

For automated mobile checks, install Python `playwright==1.63.0` in a temporary
virtual environment and run `dev/portal/browser_test.py` with that Python. It
starts/stops the simulator itself. Chrome defaults to `/usr/bin/google-chrome`;
override with `--browser PATH`. Screenshots go under ignored `.local/portal/`.
The test covers bad-password retry, a dropped begin response/status outage,
connected management with the device password, scoped deletion, local-only assets, mobile
width, and absence of secrets in URLs/browser storage.

The actual WASM build, mobile Chromium flow and Rust/private-D-Bus tests pass.
The ARM binary and offline assets are deployed to the development Pi; the binary
executes there. The updated shared-password provisioning flow remains to be deployed and checked on hardware.
Pi phone onboarding, failed-password AP restoration and return to station-LAN
management remain hardware acceptance steps. WPA2/WPA3 classification and the
broader failure matrix remain tracked in the implementation/testing plans.

Dioxus implementation references: [components](https://dioxuslabs.com/learn/0.7/essentials/ui/components/),
[async work](https://dioxuslabs.com/learn/0.7/essentials/basics/async/),
[data fetching](https://dioxuslabs.com/learn/0.7/essentials/basics/resources/), and
[local assets](https://dioxuslabs.com/learn/0.7/essentials/ui/assets/).

To verify hot reload against an already-running, freshly started `cargo make dev`,
run `dev/portal/hot_reload_test.py` with the Playwright Python environment. Use
`--url http://127.0.0.1:PORT` for a custom fixture port. The check temporarily edits
RSX strings and a CSS rule, verifies repeated edits to the badge, heading and
description in both connection states, checks form preservation and page-reload
replay, and restores its edits. Browser-local status responses leave the running
simulator's networks unchanged. The connection labels use literal component
properties: Dioxus 0.7.10 can alias repeated conditions in one RSX template and
drop edits to branches that have not rendered yet.

## Captive discovery and iPhone qualification

The iwd AP profile must advertise the AP address as DNS using
`[IPv4] DNSList=192.168.16.1`. The development provisioner now includes it;
reprovision existing test installations with the same device key and restart the
AP to update DHCP leases. Production integration must use its configured AP IP.

The daemon answers DNS on UDP and TCP port 53 at the Wi-Fi AP address only while
AwaitingCredentials. A queries resolve to that AP address with TTL zero; AAAA,
HTTPS and other types receive empty answers. It neither forwards requests nor
changes Ethernet/system DNS. DNS shuts down when onboarding leaves that state.
The HTTP service stays available on the Wi-Fi station address after connection,
with strict Host/Origin checks and no foreign-host captive redirects. Captive
discovery requires the default HTTP port 80; a custom HTTP port disables DNS.

The runtime-derived hostname or explicit `--http-hostname` overrides work in both
AP and station modes, alongside
IP access. An Origin header must use HTTP and match the request's own Host and
port, even when both names are allowed aliases. CSRF and device-password checks
also apply to hostname requests. Unknown names, wrong ports, malformed or
duplicate Host/Origin values cannot access the API. No wildcard `.local`,
forwarded-header trust, DNS lookups, or trust in network-supplied names is introduced.
Captive probe redirects still target the fixed AP IP so discovery does not
depend on mDNS availability.

Apple `/hotspot-detect.html` and `/library/test/success.html`, Android
`/generate_204`, and Windows `/connecttest.txt` requests are redirected while
captive. HTTPS is not intercepted and no false internet-success response is
sent. This uses legacy HTTP probe discovery, not DHCP option 114/CAPPORT (which
requires separate provisioning and a suitable HTTPS API endpoint).

References: [Apple's join/sign-in behavior](https://support.apple.com/en-gb/102554),
[Android's probe and CAPPORT behavior](https://developer.android.com/about/versions/11/features/captive-portal),
and [iwd AP DNS settings](https://kernel.googlesource.com/pub/scm/network/wireless/iwd/+/refs/tags/3.4/src/iwd.ap.rst).

Acceptance: forget/rejoin the setup network on the iPhone/iPad, confirm the setup
page opens inside its sign-in sheet without entering an IP address, then exercise
onboarding and failed-password retry. Automated DNS/HTTP checks validate the
server behavior; they do not establish that iOS displayed its captive UI.
