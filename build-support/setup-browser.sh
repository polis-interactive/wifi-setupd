#!/usr/bin/env bash
# Provision test-only tools on a disposable Ubuntu GitHub runner.
set -euo pipefail
: "${RUNNER_TEMP:?This installer expects a disposable GitHub runner}"
: "${GITHUB_ENV:?GitHub environment export file is required}"
sudo apt-get update
sudo apt-get install --yes dbus-daemon
python3 -m venv "$RUNNER_TEMP/browser-venv"
"$RUNNER_TEMP/browser-venv/bin/pip" install playwright==1.63.0
"$RUNNER_TEMP/browser-venv/bin/playwright" install --with-deps chromium
echo "BROWSER_PYTHON=$RUNNER_TEMP/browser-venv/bin/python" >> "$GITHUB_ENV"
echo 'PORTAL_BROWSER=chromium' >> "$GITHUB_ENV"
