# Buildroot integration handoff

This repository provides product build/install commands. BuildrootImages owns the
package recipe, image inputs, startup, D-Bus policy and device provisioning.
Read [device-provisioning.md](device-provisioning.md) alongside this document.

## Source and host tools

Fetch `https://github.com/polis-interactive/wifi-setupd.git` at an immutable
commit containing this handoff. Commit and publish the preparation changes
before choosing that revision; do not pin the preceding commit or a moving
branch. Private repository access, if required, belongs in the fetch environment.
Keep source revisions and tool download checksums in BuildrootImages.

Build from the repository root, retaining `Cargo.toml`, `Cargo.lock`,
`rust-toolchain.toml`, `.cargo/`, all four `crates/`, `build-support/` and
`LICENSE-MIT`. Cargo resolves the workspace even when compiling only the daemon.
No submodules, generated files, `.local/` settings or `dev/` files are required.
The product scripts never import or invoke the development suite.

The validated host tool set is:

| Tool | Version / requirement |
| --- | --- |
| Rust and Cargo | 1.99.0, matching `rust-toolchain.toml` |
| Rust standard libraries | `wasm32-unknown-unknown` and the selected Linux target |
| Dioxus CLI (`dx`) | 0.7.10 |
| wasm-bindgen CLI | 0.2.129, matching `Cargo.lock` |
| esbuild / Binaryen (`wasm-opt`) | 0.27.3 / 120 |
| Python | 3.9+ standard library only |
| C linker / sysroot | Buildroot SDK matching the Linux target and libc |

Supply host tools on `PATH`; `DX` or `--dx` selects the Dioxus executable,
and `CARGO` or `--cargo` selects Cargo for the daemon command. Dioxus itself
also needs Cargo/rustc on `PATH`. Supply any host-tool runtime library paths
through the build environment. There is no automatic `.local/tools` discovery
in production commands. `cargo-make` is optional convenience tooling only.

Prepare Rust targets, host tools and locked crate sources during the image
builder's download/setup phases. The image build must not run `rustup` or
`cargo install` to download missing tools. Both commands below always pass
`--locked`; use `--offline` for the actual Buildroot build. The web command sets
`NO_DOWNLOADS=1` so dx requires preinstalled asset tools. Include both WASM/UI
and daemon dependencies when preparing Cargo's cache or vendored sources;
fetching only dependencies for the ARM target is insufficient for this workspace.
Use Buildroot's Cargo source replacement configuration for vendored builds.

## Build and stage

Run these commands at the source root. Paths below are examples to replace with
the image builder's host, build and target directories. The commands do not
contact a Pi or require a running D-Bus service.

1. Build the web output with the **host/WASM environment**:

   ```sh
   python3 build-support/release.py web --offline --target-dir /build/wifi-web
   ```

   The output is `/build/wifi-web/dx/wifi-setupd-ui/release/web/public/`.
   Each build refreshes this generated public directory so obsolete hashed
   assets cannot accumulate. Do not build concurrently into the same web output.
   Dioxus generates the local assets described in its
   [0.7 asset documentation](https://dioxuslabs.com/learn/0.7/essentials/ui/assets/).

2. Build the daemon with the **target environment**, here AArch64 musl:

   ```sh
   CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=/sdk/bin/aarch64-linux-gcc \
     python3 build-support/release.py daemon --offline \
       --target aarch64-unknown-linux-musl --target-dir /build/wifi-daemon
   ```

   The executable is `/build/wifi-daemon/aarch64-unknown-linux-musl/release/wifi-setupd`.
   Target-specific linker/rustflags environment variables are honored. Keep
   generic `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, compiler wrappers and linker
   options intended for Linux out of the web environment. `--target-dir`
   overrides `CARGO_TARGET_DIR`; without either, output uses `.target/`.
   `--target` overrides `CARGO_BUILD_TARGET`; without either, daemon compilation
   uses Cargo's default target. The web command explicitly selects WASM.

3. Stage the product into Buildroot's target directory:

   ```sh
   python3 build-support/release.py install \
     --binary /build/wifi-daemon/aarch64-unknown-linux-musl/release/wifi-setupd \
     --web-root /build/wifi-web/dx/wifi-setupd-ui/release/web/public \
     --destdir /build/target
   ```

   The installer validates inputs before copying, rejects unexpected web files
   and symlinks, and refreshes only its package-owned installed web directory.
   This is a staging operation, not an atomic live-device updater. Never place
   configuration or credentials under the installed web directory.

The install allowlist is:

| Installed path | Mode | Contents |
| --- | --- | --- |
| `/usr/bin/wifi-setupd` | 0755 | Linux daemon |
| `/usr/share/wifi-setupd/web/index.html` | 0644 | Generated entry page |
| `/usr/share/wifi-setupd/web/assets/` | files 0644 | Generated JS/WASM/CSS, images and fonts |
| `/usr/share/licenses/wifi-setupd/LICENSE-MIT` | 0644 | Project license |

Buildroot should assign root ownership and traversable directory permissions.
Web files are installed alongside the daemon, not compiled into it. Pass
`--web-root /usr/share/wifi-setupd/web`. No simulator, probe, deployment helper,
SSH configuration or per-device secret is installed. Dependency license
collection remains part of Buildroot's legal-info workflow; the project license
alone does not cover every bundled dependency.

For local builds with the host tools already configured, `cargo make release`
runs the web and daemon commands. `cargo make release-web` and
`cargo make release-daemon` run them separately. The direct Python commands are
the supported interface for Buildroot and expose `--offline` explicitly.

## BuildrootImages implementation instructions

1. Add `package/wifi-setupd/Config.in`, `wifi-setupd.mk` and source/license hashes;
   source its Kconfig entry from `package/Config.in`. Use the existing external
   tree/profile tooling. Account for Rust target support, system D-Bus and iwd;
   retain iwd network configuration, required firmware and the board's working
   AP settings. Host Python and the pinned web build tools are build dependencies.
   Implement workspace-aware build/install commands using the interface above.
   Consult the selected Buildroot revision's
   [Cargo infrastructure](https://buildroot.org/downloads/manual/manual.html#_infrastructure_for_cargo_based_packages)
   for vendoring and cross-build environment support; its default single-target
   recipe does not express this project's separate web build by itself.

2. Install an image-owned BusyBox init script and system-bus policy. Start one
   daemon only after persistent storage is mounted, provisioning is complete,
   system D-Bus is available, and iwd exposes the intended Wi-Fi device. Use
   bounded readiness waits with a visible failure/retry path. The simplest
   initial service account is root, which also permits ports 53/80 and reading
   the protected device key. Restrict ownership of `tv.polis.WifiSetup1` to that
   account. Permit calls/signals for the intended appliance client accounts and
   the daemon's required iwd calls/authentication-agent callbacks; do not copy
   the development bus's permissive temporary policy into production.

3. Implement persistent device provisioning and rotation exactly as described in
   [device-provisioning.md](device-provisioning.md). Start with arguments:

   ```text
   --interface wlan0 --ap-profile <provisioned-SSID>
   --web-root /usr/share/wifi-setupd/web
   --device-key-file /data/wifi-setupd/device-key
   ```

   Keep key parent/file permissions 0700/0600 and iwd profiles private on
   persistent storage. Block startup on incomplete provisioning. SIGTERM
   requests radio cleanup; allow more than the daemon's 30-second cleanup
   deadline before any forced termination. Keep logs writable without recording
   secrets. Enable Wi-Fi mDNS advertising and reserve AP HTTP/DNS ports;
   hostname rules and DHCP gateway/DNS requirements are in the device contract.

4. Validate a clean, offline image build and inspect the installed contents.
   Use the image's matching musl target/SDK for the current Pi profile. Do not
   substitute `aarch64-unknown-linux-gnu` artifacts into a musl image. A glibc
   image needs its own matching target, linker/sysroot and runtime loader;
   glibc deployment has not been qualified. Inspect the actual ELF interpreter
   and dynamic dependencies instead of assuming static linking from its suffix.
   The daemon uses zbus directly; it does not require libdbus or iwdrs linkage.

5. Run the [failure qualification matrix](testing-plan.md) on the integrated image:
   reboot persistence, iwd/system-bus recovery, offline mobile onboarding/retry,
   UI reconnect, scoped saved-network management and interrupted provisioning.
   Arrange actual power interruption separately from SSH process restarts and
   clean reboots. Record binary/rootfs sizes and runtime memory/CPU before
   optimizing. Exclude development SSH/services, fixtures and probes from the
   production profile, even if they are present in the development profile.

## Repository checks

```sh
python3 -m unittest discover -s tests -p 'test_release.py'
cargo fmt --all -- --check
cargo package -p wifi-setupd-zbus-api --list --locked --offline
cargo tree -p wifi-setupd-zbus-api --edges normal --locked --offline
```

The API package must remain independent of the daemon, Kameo, UI and `dev/`.
Browser acceptance can target the staged files with
`dev/portal/browser_test.py --web-root /build/target/usr/share/wifi-setupd/web`
using the simulator/browser setup in [portal.md](portal.md). That test tooling
is never part of the product build or image.

Validation on 2026-10-13 passed: all five packaging tests; formatting; public API
package/dependency boundary inspection; offline WASM and AArch64 musl builds
from a product-only source copy with an initially empty build directory; and
staging installation. That source copy contained no `dev/`, `.local/` or `.git/`.
Mobile Chromium acceptance passed against the installed web files, including
onboarding, bad-password retry, lost-response recovery, connected management
and offline assets. The validated ARM executable is statically linked with no
ELF interpreter, approximately 8.1 MiB before stripping; web files occupy
approximately 2.2 MiB. Integrated-image runtime resource measurements and the
hardware failure matrix remain integration work in BuildrootImages.
