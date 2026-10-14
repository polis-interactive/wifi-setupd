# wifi-setupd

Standalone Wi-Fi provisioning for Buildroot appliances using iwd, a captive
portal, and the public `tv.polis.WifiSetup1` D-Bus API.

**Current stage: Steps 0–4 complete; Step 4 accepted by the user on 2026-10-13.
Step 5 product packaging is ready for BuildrootImages integration.**
The [Ethernet development suite](dev/pi/README.md) cross-builds, deploys, runs,
and rolls back the scaffold on the Pi; reboot/persistence checks pass.
iwd connected to Wi-Fi and obtained an address; Wi-Fi ping and SSH passed.
The test credentials were then forgotten and stayed absent after iwd restart
and Pi reboot, with development access continuing over Ethernet.
The public API now provides typed zbus proxies, stable state/error strings, and
a coherent status snapshot. Local tests use a private D-Bus and a scripted Wi-Fi
adapter with temporary storage and controllable time. The direct iwd 3.12 adapter
now supports discovery, scanning, scoped authentication, saved-profile operations,
AP/station transitions, and recovery. On the Pi, discovery, scanning, station
association/IPv4, AP startup/explicit stop, and iwd restart recovery pass.
The Pi AP uses an iwd profile with `DisableHT=true` for driver compatibility
and stays up until explicitly stopped. Phone association and DHCP pass.
AP-to-station transition timing and the WPA2/WPA3 matrix remain to be qualified.
The executable now runs the Kameo provisioner and production D-Bus service.
It allows iwd auto-connect, starts an AP after a grace window when Wi-Fi is not
ready, and reconciles cancellation before another radio operation. The HTTP
portal now supports onboarding, retry and connected-network management.
AP-only DNS and HTTP probe redirects provide captive discovery. Remaining
security-mode and failure qualification is tracked under Step 5.
See [run the portal](docs/portal.md) and [runtime configuration](docs/provisioner.md).

## Preview the UI locally

Run `cargo make dev`, then open **http://127.0.0.1:8080** for Dioxus hot reload.
`cargo make ui` serves a static release preview.
Edit [the simulator TOML](dev/portal-sim/example.toml) for fake SSIDs and accepted
passwords. No hardware or running Wi-Fi backend is needed. See
[local UI testing](docs/portal.md#local-ui-with-fake-wi-fi) for fixture options.

## Check the workspace

Cargo writes build output to `.target/`, configured in `.cargo/config.toml`.

Run from this directory with your selected Rust toolchain (current stable for
local development). There is no repository toolchain override. Install its
`rustfmt` and `clippy` components for the checks below:

`cargo make check` runs workspace build/lint/tests, packaging tests and the
independent API consumer check. `cargo make ci` also builds and stages the
release and runs mobile browser acceptance. See the
[CI tool prerequisites](docs/buildroot.md#ci-and-local-tasks) before running the
browser pipeline locally.

```sh
cargo build --workspace --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
cargo check --manifest-path tests/api-consumer/Cargo.toml --locked --offline --target-dir .target/api-consumer
cargo run -p wifi-setupd --locked --offline -- --help
```

Tests require `dbus-daemon` on PATH and permission to bind a temporary Unix
socket, with no root privileges, host bus, host network access, or Pi. The private
bus is explicitly injected and killed/reaped after each test. A sandbox that
blocks Unix sockets needs a test-execution exception, not a system-bus fallback.
Behavioral tests and a compiling doctest cover the public
contract, adapter, actor, production exporter, and their full private-bus path.
The separate API consumer has its own workspace/lockfile and is compiled only,
not run against the host system bus.

Stable dependencies are locked: zbus 5.19.0 (Tokio backend), Tokio 1.53.2,
Serde 1.0.229, nix 0.31.3 for interface-scoped address reads, plus test-only
tempfile 3.27.0 and futures-util 0.3.34. The daemon uses Kameo 0.22.2 without
remote actor features. The production daemon and adapter probe cross-build for
AArch64 musl. The Dioxus client builds to WASM with local static assets.

## Crates

| Crate | Responsibility |
| --- | --- |
| `wifi-setupd` | Linux daemon; iwd adapter, provisioning actor, HTTP and D-Bus services |
| `wifi-setupd-zbus-api` | Public D-Bus identifiers, typed proxies, states/errors, status snapshot |
| `wifi-setupd-http-api` | Internal HTTP DTOs shared by daemon and UI; no D-Bus/iwd dependency |
| `wifi-setupd-ui` | Dioxus 0.7 web assets/WASM; HTTP client of the daemon |

Default Cargo members exclude the UI to keep daemon builds independent of web
tooling. The daemon library contains the internal adapter boundary and status
watch channel. Its library is not a stable consumer API. See the
[public contract](crates/wifi-setupd-zbus-api/README.md) for wire values, Start/Stop,
cancellation, LastError, and client examples. The production exporter is backed
by the [Step 3 actor](docs/provisioner.md); tests use a private bus and fake iwd.

See the [iwd adapter design and limits](docs/iwd-adapter.md) and
[development-only probe](dev/iwd-probe/README.md) for Step 2. The probe has a
separate workspace and deployment path; it is excluded from product packages.
iwd is the only credential store. Explicit replacement discards the selected
old profile; unrelated profiles are preserved. The web server stays
running after connection and offers saved-network management in station mode.

## Working plan

Buildroot integration agents: start with [the build/install handoff](docs/buildroot.md).
`python3 build-support/release.py` exposes separate web, daemon and staging
install commands. Production builds require no `dev/` tooling or `.local/` data.
`cargo make release` is the equivalent local build convenience task.
[CI](.github/workflows/ci.yml) runs `cargo make ci` when pull requests are opened,
reopened or updated with new commits, and on pushes to `main` (including merges).
Successful `main` runs archive the tested portal for release. Other branch pushes
without an open PR and tag pushes do not trigger CI.
The [release workflow](.github/workflows/release.yml) validates the tag, requires
successful `main` CI for that exact commit, and publishes its archived portal
after verifying checksums and provenance. Release does not rebuild.
Buildroot compiles the matching daemon with its own Rust toolchain.

Production integration agents: read the [per-device password handoff](docs/device-provisioning.md).
BuildrootImages must provision one shared AP/management password per device,
independently of image builds, and support rotation without a rebuild.

1. [Pi readiness and testing](docs/testing-plan.md): first create the development
   image, establish parameterized Ethernet/SSH updates, and prove iwd can connect
   and forget test credentials on the Pi.
2. [Original requirements](docs/requirements.md): the supplied prompt, retained
   verbatim. The project uses zbus directly instead of iwdrs.

Development image profiles, overlays, and build inputs belong in `BuildrootImages`.
This repo's SSH/deployment support belongs under `dev/`, outside the product dependency graph.
Normal release builds and library consumers must not include or invoke that
tooling. Hardware work uses Ethernet/SSH without an SBC-controller dependency.

Repository Dioxus guidance lives in
[.agents/skills/dioxus-web/SKILL.md](.agents/skills/dioxus-web/SKILL.md).

## License

Licensed under the [MIT License](LICENSE-MIT).

## Contributions

External contributions are not currently accepted. Please read
[CONTRIBUTING.md](CONTRIBUTING.md) for the contribution policy.
