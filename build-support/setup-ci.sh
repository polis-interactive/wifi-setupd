#!/usr/bin/env bash
# Provision prebuilt tools on a disposable Ubuntu x86_64 GitHub runner.
set -euo pipefail
: "${RUNNER_TEMP:?This installer expects a disposable GitHub runner}"
: "${GITHUB_PATH:?GitHub PATH export file is required}"

rustup toolchain install stable --profile minimal --component rustfmt,clippy --target wasm32-unknown-unknown

tools_dir="$RUNNER_TEMP/build-tools"
mkdir -p "$tools_dir/bin"
download_dir=$(mktemp -d "$RUNNER_TEMP/build-tools-download.XXXXXX")
trap 'rm -rf "$download_dir"' EXIT

# Check hashes before extracting executable code. No cargo install/source builds.
download() {
    local url=$1 checksum=$2 destination=$3
    curl --fail --silent --show-error --location --retry 3 "$url" --output "$destination"
    echo "$checksum  $destination" | sha256sum --check
}

download https://github.com/sagiegurari/cargo-make/releases/download/0.37.24/cargo-make-v0.37.24-x86_64-unknown-linux-musl.zip \
    ff76969f0caab39c56d168b94b2540fde34609aa6562d555081818ece387b005 "$download_dir/make.zip"
unzip -q "$download_dir/make.zip" -d "$download_dir/make"
install -m 0755 "$download_dir/make/cargo-make-v0.37.24-x86_64-unknown-linux-musl/cargo-make" "$tools_dir/bin/"

download https://github.com/DioxusLabs/dioxus/releases/download/v0.7.10/dx-x86_64-unknown-linux-gnu.tar.gz \
    4363e4ed2a3f1eb7f4d38d2d59aed59ce43271c44c16b425e92c89a64761fbe7 "$download_dir/dx.tar.gz"
tar -xzf "$download_dir/dx.tar.gz" -C "$tools_dir/bin" dx

download https://github.com/wasm-bindgen/wasm-bindgen/releases/download/0.2.129/wasm-bindgen-0.2.129-x86_64-unknown-linux-musl.tar.gz \
    82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e "$download_dir/bindgen.tar.gz"
tar -xzf "$download_dir/bindgen.tar.gz" -C "$tools_dir/bin" --strip-components=1 \
    wasm-bindgen-0.2.129-x86_64-unknown-linux-musl/wasm-bindgen

download https://github.com/WebAssembly/binaryen/releases/download/version_120/binaryen-version_120-x86_64-linux.tar.gz \
    ddb097af51d1bdb17300d986b0de7d97422f1933dedb4c9eda3510e0bf4076cc "$download_dir/binaryen.tar.gz"
tar -xzf "$download_dir/binaryen.tar.gz" -C "$tools_dir/bin" --strip-components=2 \
    binaryen-version_120/bin/wasm-opt

download https://registry.npmjs.org/@esbuild/linux-x64/-/linux-x64-0.27.3.tgz \
    066e20cdb882994160e18524a552b97e03648eb9aa0c7cdf5680a6493be65ab2 "$download_dir/esbuild.tar.gz"
tar -xzf "$download_dir/esbuild.tar.gz" -C "$tools_dir/bin" --strip-components=2 package/bin/esbuild

export PATH="$tools_dir/bin:$PATH"
echo "$tools_dir/bin" >> "$GITHUB_PATH"

# Fail during setup if a release binary cannot run on this runner.
cargo make --version
dx --version
wasm-bindgen --version
wasm-opt --version
esbuild --version
