#!/usr/bin/env python3
"""Verify RSX/CSS updates in a running cargo make dev; restore temporary edits."""
import argparse
import json
from pathlib import Path
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/wifi-setupd-ui/src/pages/setup.rs"
LAYOUT = ROOT / "crates/wifi-setupd-ui/src/layout/mod.rs"
CONNECTED = ROOT / "crates/wifi-setupd-ui/src/pages/connected.rs"
CSS = ROOT / "crates/wifi-setupd-ui/assets/style.css"
BEFORE = '"Connect to Wi-Fi"'
AFTER = '"HOT RELOAD VERIFIED"'
STYLE = "\nbody { --wifi-hot-reload-check: verified; }\n"
# Distinct text in repeated `if ready` branches must remain distinct after edits.
COPY = {
    False: ("Setup",),
    True: ("Connected", "Connected to:"),
}


def assert_copy(page, ready, suffix=""):
    for selector, text in zip((".badge", ".connection-heading"), COPY[ready]):
        expect(page.locator(selector)).to_have_text(text.replace("\\n", "\n") + suffix, timeout=30000)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--url", default="http://127.0.0.1:8080")
    parser.add_argument("--browser", default="/usr/bin/google-chrome")
    args = parser.parse_args()
    assert BEFORE in SOURCE.read_text(), "Expected UI test anchor is missing"
    replacements = [(SOURCE, BEFORE, AFTER)]
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(executable_path=args.browser, headless=True, args=["--no-sandbox"])
            page = browser.new_page(viewport={"width": 390, "height": 844})
            # Control both states without changing the running simulator's networks.
            status = dict(state="AwaitingCredentials", link="Disconnected", backend="Available", error=None,
                          ap_name="Test setup", csrf="test", networks=[], saved=[], saved_available=True,
                          scan_cached=True, connected_ssid="Test network")
            page.route("**/api/status", lambda route: route.fulfill(json=status))
            sockets = []
            page.on("websocket", lambda socket: sockets.append(socket.url))
            page.goto(args.url, wait_until="domcontentloaded")
            expect(page.get_by_role("button", name="Connect device")).to_be_enabled(timeout=120000)
            page.wait_for_timeout(1000)
            assert any("/_dioxus" in url for url in sockets), "Dioxus hot-reload websocket is missing"
            assert_copy(page, False)
            page.get_by_role("combobox", name="Networks", exact=True).fill("Preserved form state")
            SOURCE.write_text(SOURCE.read_text().replace(BEFORE, AFTER, 1))
            expect(page.get_by_text("HOT RELOAD VERIFIED", exact=True)).to_be_visible(timeout=30000)
            assert_copy(page, False)
            expect(page.get_by_role("combobox", name="Networks", exact=True)).to_have_value("Preserved form state")
            # Edit each conditional label twice, including currently unmounted branches.
            for round_number in (1, 2):
                suffix = f" [hot reload {round_number}]"
                sources = {path: path.read_text() for path in (LAYOUT, CONNECTED)}
                for texts in COPY.values():
                    for text in texts:
                        old = json.dumps(text.replace("\\n", "\n") + (f" [hot reload {round_number - 1}]" if round_number > 1 else ""), ensure_ascii=False)
                        new = json.dumps(text.replace("\\n", "\n") + suffix, ensure_ascii=False)
                        path = CONNECTED if text == "Connected to:" else LAYOUT
                        source = sources[path]
                        assert old in source, f"Expected UI test anchor missing: {old}"
                        replacements.append((path, json.dumps(text.replace("\\n", "\n"), ensure_ascii=False), new))
                        source = source.replace(old, new, 1)
                        sources[path] = source
                for path, source in sources.items():
                    path.write_text(source)
                assert_copy(page, False, suffix)
                expect(page.get_by_role("combobox", name="Networks", exact=True)).to_have_value("Preserved form state")
                status.update(state="Idle", link="Ready")
                assert_copy(page, True, suffix)
                status.update(state="AwaitingCredentials", link="Disconnected")
                assert_copy(page, False, suffix)
            CSS.write_text(CSS.read_text() + STYLE)
            page.wait_for_function("getComputedStyle(document.body).getPropertyValue('--wifi-hot-reload-check').trim() === 'verified'", timeout=30000)
            expect(page.get_by_role("combobox", name="Networks", exact=True)).to_have_value("Preserved form state")
            for path, original, marker in reversed(replacements):
                path.write_text(path.read_text().replace(marker, original, 1))
            CSS.write_text(CSS.read_text().replace(STYLE, "", 1))
            expect(page.get_by_text("HOT RELOAD VERIFIED", exact=True)).not_to_be_visible(timeout=30000)
            assert_copy(page, False)
            page.wait_for_function("getComputedStyle(document.body).getPropertyValue('--wifi-hot-reload-check').trim() === ''", timeout=30000)
            # A new page must also receive the correct replayed templates.
            page.reload(wait_until="domcontentloaded")
            assert_copy(page, False)
            status.update(state="Idle", link="Ready")
            assert_copy(page, True)
            browser.close()
        print("PASS: repeated RSX edits, both connection states, CSS, form preservation, and reload replay")
    finally:
        # Remove only our temporary markers, preserving any concurrent user edits.
        for path, original, marker in reversed(replacements):
            source = path.read_text()
            restored = source.replace(marker, original, 1)
            if restored != source:
                path.write_text(restored)
        css = CSS.read_text()
        if STYLE in css:
            CSS.write_text(css.replace(STYLE, "", 1))


if __name__ == "__main__":
    main()
