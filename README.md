# wifi-setupd

Standalone Wi-Fi provisioning for Buildroot appliances using iwd, a captive
portal, and the public `tv.polis.WifiSetup1` D-Bus API.

**Current stage: Step 0 complete; public API and local test foundation are next.**
The [Ethernet development suite](dev/pi/README.md) cross-builds, deploys, runs,
and rolls back the scaffold on the Pi; reboot/persistence checks pass.
iwd connected to Wi-Fi and obtained an address; Wi-Fi ping and SSH passed.
The test credentials were then forgotten and stayed absent after iwd restart
and Pi reboot, with development access continuing over Ethernet.
The three crates compile;
the executables print scaffold messages and exit. No Wi-Fi, D-Bus service,
HTTP server, or Dioxus frontend is implemented yet.

## Check the workspace

Run from this directory with Rust 1.99.0:

```sh
cargo build --workspace --locked --offline
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
cargo run -p wifi-setupd --locked --offline
```

The scaffold has no external dependencies and no behavioral tests yet.
`Cargo.lock` is tracked. Runtime crates will be selected and locked as each
component is introduced. The development suite also verifies AArch64 musl
builds on the Pi. WASM remains deferred; these commands check native scaffolds only.

## Crates

| Crate | Responsibility |
| --- | --- |
| `wifi-setupd` | Linux daemon; iwd adapter, provisioning actor, HTTP and D-Bus services |
| `wifi-setupd-api` | Public D-Bus identifiers, then typed proxies and public types |
| `wifi-setupd-ui` | Dioxus 0.7 web assets/WASM; HTTP client of the daemon |

Default Cargo members exclude the UI to keep daemon builds independent of web
tooling. Add the daemon's `wifi`, `provisioning`, `dbus`, and `http` modules with
their implementations rather than creating empty modules now.

## Working plan

1. [Pi readiness and testing](docs/testing-plan.md): first create the development
   image, establish parameterized Ethernet/SSH updates, and prove iwd can connect
   and forget test credentials on the Pi.
2. [Implementation milestones](docs/implementation-plan.md): Step 0 must pass
   before application implementation; iwd integration uses zbus directly.
3. [Original requirements](docs/requirements.md): the supplied prompt, retained
   verbatim. The current milestones supersede its iwdrs evaluation requirement.

Development image profiles, overlays, and build inputs belong in `BuildrootImages`.
This repo's SSH/deployment support belongs under `dev/`, outside the product dependency graph.
Normal release builds and library consumers must not include or invoke that
tooling. Hardware work uses Ethernet/SSH without an SBC-controller dependency.

Repository Dioxus guidance lives in
[skills/dioxus-web/SKILL.md](skills/dioxus-web/SKILL.md).
