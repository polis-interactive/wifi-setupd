#!/usr/bin/env bash
# Provision build tools on a disposable Ubuntu GitHub runner; never part of an image build.
set -euo pipefail
: "${RUNNER_TEMP:?This installer expects a disposable GitHub runner}"
: "${GITHUB_PATH:?GitHub PATH export file is required}"
rustup toolchain install stable --profile minimal --component rustfmt,clippy --target wasm32-unknown-unknown
cargo install cargo-make --version 0.37.24 --locked
cargo install dioxus-cli --version 0.7.10 --locked
cargo install wasm-bindgen-cli --version 0.2.129 --locked
npm install --prefix "$RUNNER_TEMP/web-tools" esbuild@0.27.3
echo "$RUNNER_TEMP/web-tools/node_modules/.bin" >> "$GITHUB_PATH"
curl --fail --location --retry 3 \
  https://github.com/WebAssembly/binaryen/releases/download/version_120/binaryen-version_120-x86_64-linux.tar.gz \
  --output "$RUNNER_TEMP/binaryen.tar.gz"
tar -xzf "$RUNNER_TEMP/binaryen.tar.gz" -C "$RUNNER_TEMP"
echo "$RUNNER_TEMP/binaryen-version_120/bin" >> "$GITHUB_PATH"
