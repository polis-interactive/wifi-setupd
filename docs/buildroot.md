# Buildroot integration handoff

This repository provides product build/install commands. BuildrootImages owns the
package recipe, image inputs, startup, D-Bus policy and device provisioning.
Read [device-provisioning.md](device-provisioning.md) alongside this document.

The current consumer contract is
[BuildrootImages/package/polis-wifi-setupd/README.md](../../BuildrootImages/package/polis-wifi-setupd/README.md).
Buildroot compiles the daemon with its own Cargo/Rust and downloads the matching
prebuilt portal from the GitHub release. Image builds need no Dioxus, WASM
compiler or rustup installation.

## Rust toolchain policy

There is no `rust-toolchain.toml`: local builds use the caller's selected
toolchain, and Buildroot uses its directly invoked Cargo/rustc. CI uses the
current stable channel and records the exact compiler version in provenance.
Release reuses that build without installing a Rust toolchain.
This avoids synchronizing an exact compiler pin across repositories. Locked
dependencies remain in `Cargo.lock`; this does not promise compatibility with
every older compiler or declare a minimum supported Rust version.

The current daemon and locked dependencies were successfully cross-compiled
with Buildroot's Rust 1.97.1 and its AArch64 musl SDK on 2026-10-13. The resulting
ELF is statically linked. That is compatibility evidence, not a toolchain pin.
Buildroot should continue checking compilation when either repository changes.

## Release artifacts and publication

[CI](../.github/workflows/ci.yml) builds and tests opened, reopened and updated
pull requests, plus pushes to `main` (including merges). Other branch pushes
without an open PR and tag pushes do not trigger CI. After a successful `main`
build and browser test, `cargo make package-web` archives the staged portal
without rebuilding it, using `workspace.package.version`. CI uploads it as
`release-portal-COMMIT-ATTEMPT`, retained for 90 days. PR runs do not upload
release artifacts. New PR updates cancel older PR runs; each main commit has
its own concurrency group so later merges do not cancel its build.

[Release](../.github/workflows/release.yml) runs when a GitHub release is
published, including prereleases. It validates that the tag matches
`workspace.package.version` and its original commit, then requires successful
push CI on `main` for that exact commit. It waits for the latest matching run
if pending; missing, failed or cancelled CI blocks release. It downloads the
artifact from that successful run attempt and verifies checksums, source commit,
version, clean-source status and Cargo.lock hash before publishing the same bytes.
Release performs no compilation or tests and installs no build tools.

Make version changes in a PR, merge to `main`, let CI pass, then tag that tested
commit and publish the release. Missing, expired or mismatched artifacts block
publication; there is no automatic rebuild fallback. If the artifact has expired,
rerun the entire main CI run for that commit before retrying release. Each run
attempt uses a distinct artifact name, preventing reuse of an older attempt's
build. Retention is also subject to the repository's Actions settings.

Release publishes these assets:

| Asset | Contents |
| --- | --- |
| `wifi-setupd-web-VERSION.tar.gz` | `index.html` and `assets/` at the archive root |
| `wifi-setupd-web-VERSION.sha256` | SHA-256 for the archive and provenance file |
| `wifi-setupd-web-VERSION.provenance.json` | Source commit, Cargo.lock hash, exact web/Rust tool versions, archive hash and source timestamp |

Use numeric tags such as `0.1.1` or `0.1.1-rc.1`, without a `v` prefix, matching
the Buildroot asset naming contract. The existing `0.1.0` tag predates this
workflow; bump the workspace version, refresh affected lockfiles and publish a
new patch tag containing the workflow. Advance Buildroot's daemon source and
portal version together.
Record the published portal's actual hash in `polis-wifi-setupd.hash`; the
Buildroot-vendored source archive has its own separate hash.

The archive has sorted entries, normalized ownership and permissions, and a
timestamp taken from the source commit in both tar and gzip metadata. This makes
packaging identical web bytes repeatable; changing the stable Rust compiler can
still change rebuilt WASM. Release uploads without `--clobber` and will not replace
existing assets. Fix a published release by publishing a new version. The
workflow requires release asset uploads to remain enabled after publication;
GitHub's [immutable-release setting](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases)
instead requires a draft-first upload flow.

To package already-tested web output locally:

```sh
python3 build-support/archive-web.py \
  --web-root .target/release-root/usr/share/wifi-setupd/web \
  --version 0.1.1 --output-dir dist
```

Use a fresh output directory. Local provenance reports tracked source changes;
the release task rejects such a dirty build. Only the archive, checksum and
provenance are published; Buildroot builds the target daemon from the same
release's source.

## CI and local tasks

The workflows orchestrate checked-in commands; no inline Python or test logic
lives in YAML. The same tasks can run locally:

| Command | Purpose |
| --- | --- |
| `cargo make check` | Fetch locked dependencies; format, build, Clippy, Rust/packaging/release-validation tests and API consumer check |
| `cargo make ci` | All checks, release build/staging and mobile browser acceptance |
| `cargo make package-web` | Archive already-tested staging output using the workspace version, without rebuilding |
| `cargo make release-stage` | Build and install into a staging root |
| `cargo make test-browser` | Build the simulator and test an already-staged portal |
| `RELEASE_TAG=0.1.1 cargo make release-package` | Build, stage and archive a clean release checkout |

Install cargo-make 0.37.24 and the web tools below. Tests also need `dbus-daemon`
and a Python environment with `playwright==1.63.0`. Set `BROWSER_PYTHON` to that
environment's Python. `PORTAL_BROWSER` defaults to `/usr/bin/google-chrome`;
set it to `chromium` to use Playwright's installed browser, or to another browser
executable path. Tasks never install browser/system packages on a developer's
machine. For isolated GitHub Ubuntu runners, `build-support/setup-ci.sh` installs
build tools and `build-support/setup-browser.sh` installs test tools. Release
needs neither installer.

The staging tasks default to native output under `.target/`, staged into
`.target/release-root`, with archives under `dist/`. Override `RELEASE_BINARY`,
`RELEASE_WEB_ROOT`, `RELEASE_DESTDIR` and `RELEASE_OUTPUT_DIR` for other layouts;
when changing Cargo's target or output directory, supply the matching artifact
paths. Individual `release-web`/`release-daemon` tasks expect dependencies to be
prefetched (`cargo make fetch`). Buildroot continues using the explicit Python
build/install interface below, without a cargo-make dependency.

## Source and host tools

Fetch `https://github.com/polis-interactive/wifi-setupd.git` at an immutable
commit containing this handoff. Commit and publish the preparation changes
before choosing that revision; do not pin the preceding commit or a moving
branch. Private repository access, if required, belongs in the fetch environment.
Keep source revisions and tool download checksums in BuildrootImages.

Build from the repository root, retaining `Cargo.toml`, `Cargo.lock`,
`.cargo/`, all four `crates/`, `build-support/` and
`LICENSE-MIT`. Cargo resolves the workspace even when compiling only the daemon.
No submodules, generated files, `.local/` settings or `dev/` files are required.
The product scripts never import or invoke the development suite.

The web release build uses the following host tools. Buildroot only needs its
Rust/SDK and Python when consuming the published portal:

| Tool | Version / requirement |
| --- | --- |
| Rust and Cargo | Caller-selected; current stable in CI |
| Rust standard libraries | `wasm32-unknown-unknown` and the selected Linux target |
| Dioxus CLI (`dx`) | 0.7.10 |
| wasm-bindgen CLI | 0.2.129, matching `Cargo.lock` |
| esbuild / Binaryen (`wasm-opt`) | 0.27.3 / 120 |
| Python | 3.9+ for product build/install; 3.11+ for archiving, release validation and CI tests |
| C linker / sysroot | Buildroot SDK matching the Linux target and libc |

Supply host tools on `PATH`; `DX` or `--dx` selects the Dioxus executable,
and `CARGO` or `--cargo` selects Cargo for the daemon command. Dioxus itself
also needs Cargo/rustc on `PATH`. Supply any host-tool runtime library paths
through the build environment. There is no automatic `.local/tools` discovery
in production commands. `cargo-make` is optional convenience tooling only.
For rustup-based local web builds, install the WASM target into your selected
toolchain with `rustup target add wasm32-unknown-unknown`; deleting the override
does not transfer installed targets between toolchains.

Prepare Rust targets, host tools and locked crate sources during the relevant
builder's download/setup phases. CI installs web tools before building;
the Buildroot image build must not run `rustup` or `cargo install` to download
missing tools. Both commands below always pass
`--locked`; use `--offline` for the actual Buildroot build. The web command sets
`NO_DOWNLOADS=1` so dx requires preinstalled asset tools. Include both WASM/UI
and daemon dependencies when preparing Cargo's cache or vendored sources;
fetching only dependencies for the ARM target is insufficient for this workspace.
Use Buildroot's Cargo source replacement configuration for vendored builds.

## Build and stage

These commands support local builds and the CI build used for release. Buildroot's existing package
replaces the web build with download, hash verification and extraction of the
release portal, then uses the daemon and install commands below.
Run them at the source root. Paths below are examples to replace with
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

1. Maintain `package/polis-wifi-setupd/Config.in`, `polis-wifi-setupd.mk` and
   source/license/portal hashes using the existing external tree/profile tooling.
   Account for Rust target support, system D-Bus and iwd;
   retain iwd network configuration, required firmware and the board's working
   AP settings. Host Python is a build dependency; web tools run in upstream CI.
   Use workspace-aware build/install commands and the matching release portal.
   Consult the selected Buildroot revision's
   [Cargo infrastructure](https://buildroot.org/downloads/manual/manual.html#_infrastructure_for_cargo_based_packages)
   for vendoring and cross-build environment support.

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
python3 -m unittest discover -s tests -p 'test_*.py'
cargo fmt --all -- --check
cargo package -p wifi-setupd-zbus-api --list --locked --offline
cargo tree -p wifi-setupd-zbus-api --edges normal --locked --offline
```

The API package must remain independent of the daemon, Kameo, UI and `dev/`.
Browser acceptance can target the staged files with
`dev/portal/browser_test.py --web-root /build/target/usr/share/wifi-setupd/web`
using the simulator/browser setup in [portal.md](portal.md). That test tooling
is never part of the product build or image.

Validation on 2026-10-13 passed: packaging tests; formatting; public API
package/dependency boundary inspection; offline WASM and AArch64 musl builds
from a product-only source copy with an initially empty build directory; and
staging installation. That source copy contained no `dev/`, `.local/` or `.git/`.
Mobile Chromium acceptance passed against the installed web files, including
onboarding, bad-password retry, lost-response recovery, connected management
and offline assets. The validated ARM executable is statically linked with no
ELF interpreter, approximately 8.1 MiB before stripping; web files occupy
approximately 2.2 MiB. Integrated-image runtime resource measurements and the
hardware failure matrix remain integration work in BuildrootImages.

Release automation validation also passes: packaging/archive tests,
actionlint, checksum/provenance checks, extraction with BuildrootImages'
`extract-web.py`, and mobile browser acceptance of that extracted archive.
The GitHub-hosted workflow and upload still require a new published release;
local verification did not publish assets.

The split task entry points were also exercised locally: `cargo make ci` passed
workspace compilation, formatting, Clippy, Rust tests, 11 packaging/validation
tests, API consumer compilation and mobile browser acceptance. A clean temporary
product-only source snapshot passed `cargo make release-package`; a dirty source
tree was rejected before packaging. Both workflows pass actionlint and the
runner setup scripts pass Bash syntax checks. Live GitHub CI gating and publishing
remain unexercised until these workflows are pushed.
