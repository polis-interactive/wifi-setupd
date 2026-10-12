#!/usr/bin/env python3
"""Exercise the real WASM UI, HTTP server and actor with an in-memory radio."""
import argparse
from pathlib import Path
import subprocess
import time
import urllib.request
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", default="/usr/bin/google-chrome")
    args = parser.parse_args()
    root = ROOT / ".target/dx/wifi-setupd-ui/release/web/public"
    if not (root / "index.html").is_file():
        raise RuntimeError("Build the Dioxus release assets first")
    server = subprocess.Popen([str(ROOT / ".target/portal-sim/debug/wifi-setupd-portal-sim"), str(root)])
    try:
        for _ in range(100):
            if server.poll() is not None:
                raise RuntimeError("Simulator exited")
            try:
                urllib.request.urlopen("http://127.0.0.1:8080/api/status", timeout=1).close()
                break
            except OSError:
                time.sleep(0.1)
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(executable_path=args.browser, headless=True, args=["--no-sandbox"])
            context = browser.new_context(viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True)
            page = context.new_page()
            exceptions = []
            external = []
            submissions = []
            requested_urls = []
            page.on("request", lambda request: requested_urls.append(request.url))
            page.on("pageerror", lambda error: exceptions.append(str(error)))
            page.on("request", lambda request: submissions.append(request.url) if request.method == "POST" and request.url.endswith("/api/connect") else None)
            def local_only(route):
                if route.request.url.startswith("http://127.0.0.1:8080/"):
                    route.continue_()
                else:
                    external.append(route.request.url)
                    route.abort()
            page.route("**/*", local_only)
            page.goto("http://127.0.0.1:8080/")
            expect(page.get_by_role("button", name="Connect device")).to_be_enabled(timeout=15000)
            page.wait_for_function("getComputedStyle(document.body).fontFamily.includes('Inter')")
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "mobile overflow"
            output = ROOT / ".local/portal"
            output.mkdir(parents=True, exist_ok=True)
            page.screenshot(path=str(output / "onboarding.png"), full_page=True)
            page.get_by_label("Network name", exact=True).fill("Home Wi-Fi")
            page.get_by_label("Wi-Fi password", exact=True).fill("wrong-password")
            page.get_by_role("button", name="Connect device").click()
            expect(page.get_by_role("heading", name="Joining your network")).to_be_visible()
            expect(page.get_by_text("The network rejected the password.", exact=False)).to_be_visible(timeout=15000)
            expect(page.get_by_label("Wi-Fi password", exact=True)).to_have_value("")
            assert len(submissions) == 1
            page.screenshot(path=str(output / "retry.png"), full_page=True)
            # Deliver begin to the server, but lose its response and subsequent
            # status polls, as when a single radio tears down the phone's AP.
            def lose_begin(route):
                route.fetch()
                page.route("**/api/status", lambda r: r.abort())
                route.abort()
            page.route("**/begin", lose_begin)
            page.get_by_label("Wi-Fi password", exact=True).fill("working-password")
            page.get_by_role("button", name="Connect device").click()
            expect(page.get_by_role("heading", name="Joining your network")).to_be_visible()
            expect(page.get_by_text("Connection interrupted.", exact=False)).to_be_visible(timeout=15000)
            expect(page.get_by_text("The network rejected the password.", exact=False)).not_to_be_visible()
            assert len(submissions) == 2
            page.screenshot(path=str(output / "switching.png"), full_page=True)
            page.unroute("**/api/status")
            expect(page.get_by_role("heading", name="You’re connected.")).to_be_visible(timeout=15000)
            row = page.locator(".network").filter(has_text="Old network")
            row.get_by_role("button", name="Forget", exact=True).click()
            page.get_by_role("button", name="Confirm forget").click()
            expect(page.get_by_text("Enter the device management key.", exact=True)).to_be_visible()
            page.get_by_label("Device management key", exact=True).fill("simulator-management-key-for-testing")
            page.get_by_role("button", name="Confirm forget").click()
            expect(row).to_have_count(0, timeout=15000)
            expect(page.locator(".network").filter(has_text="Home Wi-Fi")).to_have_count(1)
            page.get_by_label("Device management key", exact=True).fill("")
            assert page.evaluate("localStorage.length === 0 && sessionStorage.length === 0")
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            page.screenshot(path=str(output / "connected.png"), full_page=True)
            assert all("password" not in url for url in requested_urls), "secret entered a URL"
            assert not exceptions, exceptions
            assert not external, external
            browser.close()
        print("PASS: mobile onboarding, bad-password retry, lost response, connected management, offline assets, no browser secret storage")
    finally:
        server.terminate()
        server.wait(timeout=10)


if __name__ == "__main__":
    main()
