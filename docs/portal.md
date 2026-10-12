# Local Wi-Fi portal (Step 4)

The daemon serves a Dioxus 0.7.10 web app through Axum. It stays running across
AP/station transitions, binding port 80 only to IPv4 addresses on the configured
Wi-Fi interface. Ethernet addresses are not selected. Start with direct local-IP
navigation; automatic captive-portal discovery and DNS interception are not
implemented or claimed.

## Try it on the Pi

The development runtime has a separate binary/assets deployment, leaving image
startup integration for Step 5. Run from the repository root:

```sh
python3 dev/pi/portal.py build
python3 dev/pi/portal.py deploy
python3 dev/pi/portal.py start
```

`build` requires `dx` 0.7.10 and the `wasm32-unknown-unknown` Rust target; use
`--dx /path/to/dx` if it is not on PATH. Cross-linker/SSH settings come from
ignored `.local/pi/config.toml`. The current checkout also has an ignored
`.local/tools/` tool bundle, which the helper automatically uses for offline
rebuilds (dx 0.7.10, wasm-bindgen 0.2.129, esbuild 0.27.3 and Binaryen 120).
These are development tools, never product dependencies or shipped assets.

On first start, enter the **Pi setup AP passphrase**, not the router password.
Use the passphrase already used for `wifi-setupd-test` so the phone can rejoin
without changing its saved setup-network password. The helper creates a private
iwd AP profile only if none exists; it never reads or overwrites an existing one.
This development fixture uses `DisableHT=true` and `192.168.16.1/28` so failed
attempts restore the same setup address. The PSK persists only in iwd storage;
there is no app credential database. Production image provisioning remains owned
by BuildrootImages. The AP settings follow [iwd's AP profile format](https://kernel.googlesource.com/pub/scm/network/wireless/iwd/+/refs/tags/3.4/src/iwd.ap.rst).

Join `wifi-setupd-test` on the phone and open **http://192.168.16.1**. Enter the
router SSID and password in that page. The AP has no lifetime/shutdown timer.
After success, join the router's Wi-Fi and open the Pi's new Wi-Fi IP address;
`python3 dev/pi/portal.py status` prints its current IPv4 address over Ethernet.
The page changes to connected-network management. If connection fails, rejoin
the setup AP and reopen its address to retry.

Other development commands:

```sh
python3 dev/pi/portal.py logs
python3 dev/pi/portal.py stop
```

For an update, build/deploy, explicitly stop the old runtime, then start the new
one. The current runtime keeps its existing release binary while an update is
uploaded. This helper does not install a boot service or change image definitions.

## Connection and deletion behavior

The UI displays cached scan results collected before AP startup, with manual
SSID entry for missing networks. Adopting an already-running AP may leave this
list empty. It does not scan a single radio while the phone is using the AP.

The HTTP flow has two phases: `POST /api/connect` validates and reserves a
request, returning a random attempt ID without changing the radio. After
receiving acceptance, the browser clears its password field and calls
`POST /api/connect/{id}/begin`. Only then does the actor tear down the AP and
connect. Repeating begin for the last accepted attempt is idempotent. An unstarted
reservation expires after 30 seconds and drops its credentials; that timer
never stops the AP. Requests remain only in memory, and iwd persists successful
credentials. Selecting replacement explicitly permits discarding the same
network's saved profile, without restoring its previous secret on failure.

A lost response means the outcome is unknown, not that the password was rejected.
The UI explains how to rejoin either network and polls again when reachable.
Backend authentication errors get specific wording only when the adapter has
positive evidence; generic iwd failures remain generic errors.

`GET /api/status` exposes state, cached networks, saved-network metadata and a
CSRF token; no Wi-Fi secrets. `POST /api/forget` requires the device management
key and a selected SSID/security pair currently known to iwd. Deletion goes
through the actor and iwd, preserving unrelated profiles. Deleting a currently
used network may disconnect Wi-Fi and trigger automatic AP fallback.

The development deployer generates a private management key at
`.local/pi/portal-management-key`. Read it locally when using the connected UI;
never put it in a URL. The browser holds it only in memory. Production operators
provide a protected key file through `--management-key-file`; omitting this
option disables HTTP deletion. This key is separate from every Wi-Fi password.

All mutations require a per-listener CSRF token; Host must match the bound IP
and port, and any Origin must match it exactly. Bodies are capped at 4 KiB and
errors do not echo payloads. Responses use no-store and deny framing. HTTP is
intended for the protected setup AP/trusted local Wi-Fi; transport encryption,
remote access, hostnames, and device discovery are outside this slice.

## Local browser acceptance

```sh
rustup target add wasm32-unknown-unknown
cargo install dioxus-cli --version 0.7.10 --locked
dx build --platform web --release --package wifi-setupd-ui --locked --offline
cargo build --manifest-path dev/portal-sim/Cargo.toml --locked --offline --target-dir .target/portal-sim
```

Run `.target/portal-sim/debug/wifi-setupd-portal-sim
 .target/dx/wifi-setupd-ui/release/web/public` (on one line) and open
http://127.0.0.1:8080. The simulator never accesses D-Bus, host Wi-Fi, or persistent
credentials. `wrong-password` simulates authentication rejection; another valid
password succeeds. Its management key is `simulator-management-key-for-testing`.

For automated mobile checks, install Python `playwright==1.63.0` in a temporary
virtual environment and run `dev/portal/browser_test.py` with that Python. It
starts/stops the simulator itself. Chrome defaults to `/usr/bin/google-chrome`;
override with `--browser PATH`. Screenshots go under ignored `.local/portal/`.
The test covers bad-password retry, a dropped begin response/status outage,
connected management authorization, scoped deletion, local-only assets, mobile
width, and absence of secrets in URLs/browser storage.

The actual WASM build, mobile Chromium flow and Rust/private-D-Bus tests pass.
The ARM binary and offline assets are deployed to the development Pi; the binary
executes there. Runtime startup awaits the local AP-passphrase prompt.
Pi phone onboarding, failed-password AP restoration and return to station-LAN
management remain hardware acceptance steps. WPA2/WPA3 classification and the
broader failure matrix remain tracked in the implementation/testing plans.

Dioxus implementation references: [components](https://dioxuslabs.com/learn/0.7/essentials/ui/components/),
[async work](https://dioxuslabs.com/learn/0.7/essentials/basics/async/),
[data fetching](https://dioxuslabs.com/learn/0.7/essentials/basics/resources/), and
[local assets](https://dioxuslabs.com/learn/0.7/essentials/ui/assets/).
