#!/usr/bin/env python3
"""Exercise the real WASM UI, HTTP server and actor with an in-memory radio."""
import argparse
from pathlib import Path
import subprocess
import time
import tempfile
import urllib.request
from playwright.sync_api import sync_playwright, expect

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", default="/usr/bin/google-chrome",
                        help="Browser executable, or 'chromium' for Playwright's managed browser")
    parser.add_argument("--web-root", type=Path,
                        default=ROOT / ".target/dx/wifi-setupd-ui/release/web/public")
    args = parser.parse_args()
    root = args.web_root.resolve()
    if not (root / "index.html").is_file():
        raise RuntimeError("Build the Dioxus release assets first")
    fixture = tempfile.TemporaryDirectory(prefix="wifi-portal-browser-")
    config = Path(fixture.name) / "config.toml"
    config.write_text((ROOT / "dev/portal-sim/example.toml").read_text().replace("port = 8080", "port = 18080", 1))
    server = subprocess.Popen([str(ROOT / ".target/portal-sim/debug/wifi-setupd-portal-sim"), str(root), str(config)])
    try:
        for _ in range(100):
            if server.poll() is not None:
                raise RuntimeError("Simulator exited")
            try:
                urllib.request.urlopen("http://127.0.0.1:18080/api/status", timeout=1).close()
                break
            except OSError:
                time.sleep(0.1)
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(
                executable_path=None if args.browser == "chromium" else args.browser,
                headless=True, args=["--no-sandbox"])
            context = browser.new_context(viewport={"width": 390, "height": 844}, is_mobile=True, has_touch=True)
            page = context.new_page()
            exceptions = []
            external = []
            submissions = []
            requested_urls = []
            replacements = []
            page.on("request", lambda request: requested_urls.append(request.url))
            page.on("pageerror", lambda error: exceptions.append(str(error)))
            page.on("request", lambda request: submissions.append(request.url) if request.method == "POST" and request.url.endswith("/api/connect") else None)
            page.on("request", lambda request: replacements.append(request.post_data_json["replace"]) if request.method == "POST" and request.url.endswith("/api/connect") else None)
            def local_only(route):
                if route.request.url.startswith("http://127.0.0.1:18080/"):
                    route.continue_()
                else:
                    external.append(route.request.url)
                    route.abort()
            page.route("**/*", local_only)
            page.goto("http://127.0.0.1:18080/")
            expect(page.get_by_role("button", name="Connect device")).to_be_enabled(timeout=15000)
            page.wait_for_function("getComputedStyle(document.body).fontFamily.includes('Inter')")
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "mobile overflow"
            output = ROOT / ".local/portal"
            output.mkdir(parents=True, exist_ok=True)
            page.screenshot(path=str(output / "onboarding.png"), full_page=True)
            expect(page.locator(".intro")).to_have_count(0)
            expect(page.get_by_label("Network name", exact=True)).to_have_count(0)
            expect(page.get_by_text("Found before setup started.", exact=False)).to_have_count(0)
            expect(page.get_by_label("Replace any saved credentials", exact=False)).to_have_count(0)
            picker = page.get_by_role("combobox", name="Networks", exact=True)
            page.get_by_role("button", name="Show networks", exact=True).tap()
            expect(picker).to_be_focused()
            page.get_by_role("option", name="Workshop", exact=True).tap()
            expect(picker).to_have_value("Workshop")
            picker.fill("Hidden network")
            page.get_by_role("option", name="Use “Hidden network”", exact=True).tap()
            expect(picker).to_have_value("Hidden network")
            picker.fill("Home")
            picker.press("ArrowDown")
            expect(picker).to_have_attribute("aria-activedescendant", "network-option-0")
            picker.press("Enter")
            expect(picker).to_have_value("Home Wi-Fi")
            expect(picker).to_have_attribute("aria-expanded", "false")
            picker.press("ArrowDown")
            picker.press("Escape")
            expect(picker).to_have_attribute("aria-expanded", "false")
            assert not submissions, "choosing a network must not submit the form"
            page.get_by_label("Wi-Fi password", exact=True).fill("wrong-password")
            page.get_by_role("button", name="Connect device").click()
            expect(page.get_by_role("heading", name="Joining your network")).to_be_visible()
            expect(page.get_by_text("The network rejected the password.", exact=False)).to_be_visible(timeout=15000)
            expect(page.get_by_label("Wi-Fi password", exact=True)).to_have_value("")
            assert len(submissions) == 1
            page.screenshot(path=str(output / "retry.png"), full_page=True)
            rejection = page.get_by_text("The network rejected the password.", exact=False)
            # Dismiss outside the form; identical backend polls must not revive it.
            page.locator("header").click()
            expect(rejection).not_to_be_visible()
            for _ in range(2):
                with page.expect_response("**/api/status"):
                    pass
            expect(rejection).not_to_be_visible()
            # A new failure with the same error must still appear.
            page.get_by_label("Wi-Fi password", exact=True).fill("wrong-password")
            page.get_by_role("button", name="Connect device").click()
            expect(rejection).to_be_visible(timeout=15000)
            page.keyboard.press("Tab")
            expect(rejection).not_to_be_visible()
            with page.expect_response("**/api/status"):
                pass
            expect(rejection).not_to_be_visible()
            page.get_by_label("Wi-Fi password", exact=True).fill("wrong-password")
            page.get_by_role("button", name="Connect device").click()
            expect(rejection).to_be_visible(timeout=15000)
            page.get_by_label("Wi-Fi password", exact=True).fill("working-password")
            expect(rejection).not_to_be_visible()
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
            assert len(submissions) == 4
            assert replacements == [True] * 4, "portal submissions always replace saved credentials"
            page.screenshot(path=str(output / "switching.png"), full_page=True)
            page.unroute("**/api/status")
            expect(page.get_by_role("heading", name="Connected to:", exact=False)).to_be_visible(timeout=15000)
            expect(page.locator(".connected-ssid")).to_have_text("Home Wi-Fi")
            expect(page.locator(".lede")).to_have_count(0)
            # A fresh browser document must learn the SSID from status.
            page.reload()
            expect(page.locator(".connected-ssid")).to_have_text("Home Wi-Fi", timeout=15000)
            row = page.locator(".network").filter(has_text="Workshop")
            forget = row.get_by_role("button", name="Forget", exact=True)
            forget.click()
            modal = page.get_by_role("dialog", name="Forget Workshop?", exact=True)
            expect(modal).to_be_visible()
            assert modal.evaluate("dialog => dialog.matches(':modal')"), "confirmation must block the background"
            expect(modal.get_by_label("Device password", exact=True)).to_be_focused()
            forget.evaluate("button => button.focus()")
            expect(modal.get_by_label("Device password", exact=True)).to_be_focused()
            page.keyboard.press("Tab")
            expect(modal.get_by_role("button", name="Cancel", exact=True)).to_be_focused()
            page.keyboard.press("Tab")
            expect(modal.get_by_role("button", name="Confirm forget")).to_be_focused()
            page.keyboard.press("Shift+Tab")
            expect(modal.get_by_role("button", name="Cancel", exact=True)).to_be_focused()
            page.keyboard.press("Escape")
            expect(modal).not_to_be_visible()
            expect(forget).to_be_focused()
            expect(row).to_have_count(1)
            forget.click()
            modal.get_by_role("button", name="Cancel", exact=True).click()
            expect(modal).not_to_be_visible()
            expect(forget).to_be_focused()
            forget.click()
            page.mouse.click(4, 4)
            expect(modal).not_to_be_visible()
            expect(row).to_have_count(1)
            forget.click()
            page.screenshot(path=str(output / "forget-modal.png"), full_page=True)
            modal.get_by_role("button", name="Confirm forget").click()
            expect(modal.get_by_text("Enter the device password used to join the setup Wi-Fi.", exact=True)).to_be_visible()
            page.get_by_label("Device password", exact=True).fill("wrong-device-password")
            with page.expect_response("**/api/forget") as denied:
                page.get_by_role("button", name="Confirm forget").click()
            assert denied.value.status == 401
            expect(row).to_have_count(1)
            page.get_by_label("Device password", exact=True).fill("simulator-device-password")
            pending_forget = []
            page.route("**/api/forget", lambda route: pending_forget.append(route))
            page.get_by_role("button", name="Confirm forget").click()
            expect(modal).not_to_be_visible()
            expect(row).to_have_count(0, timeout=1000)
            # Polls arriving before the POST response must not bring the row back.
            page.wait_for_timeout(2200)
            expect(row).to_have_count(0)
            assert len(pending_forget) == 1
            pending_forget[0].fulfill(response=pending_forget[0].fetch())
            page.unroute("**/api/forget")
            expect(page.get_by_text("Network deletion requested.", exact=True)).to_have_count(0)
            expect(page.locator(".network").filter(has_text="Home Wi-Fi")).to_have_count(1)
            expect(page.get_by_label("Device management key", exact=True)).to_have_count(0)
            assert page.evaluate("localStorage.length === 0 && sessionStorage.length === 0")
            assert page.evaluate("document.documentElement.scrollWidth <= innerWidth")
            page.screenshot(path=str(output / "connected.png"), full_page=True)
            current = page.locator(".network").filter(has_text="Home Wi-Fi")
            stale_status = page.request.get("http://127.0.0.1:18080/api/status").json()
            pending_active = []
            page.route("**/api/forget", lambda route: pending_active.append(route))
            current.get_by_role("button", name="Forget", exact=True).click()
            page.get_by_role("dialog", name="Forget Home Wi-Fi?", exact=True).get_by_role("button", name="Confirm forget").click()
            reconnect = page.get_by_role("heading", name="Reconnect to setup Wi-Fi", exact=True)
            expect(reconnect).to_be_visible(timeout=1000)
            expect(page.get_by_role("heading", name="Connected to:", exact=False)).not_to_be_visible()
            expect(page.locator(".badge")).to_have_text("Disconnecting")
            page.wait_for_timeout(2200)
            expect(reconnect).to_be_visible()
            # A definite rejection rolls back the optimistic transition.
            pending_active.pop().fulfill(status=401, content_type="application/json", body='{"error":"Wrong device password"}')
            modal = page.get_by_role("dialog", name="Forget Home Wi-Fi?", exact=True)
            expect(modal).to_be_visible()
            expect(page.get_by_role("heading", name="Connected to:", exact=False)).to_be_visible()
            expect(modal.get_by_text("Wrong device password", exact=True)).to_be_visible()
            # Stale Ready polls and a lost deletion response must not restore
            # the Connected screen after the request actually disconnects us.
            page.route("**/api/status", lambda route: route.fulfill(json=stale_status))
            modal.get_by_role("button", name="Confirm forget").click()
            expect(reconnect).to_be_visible(timeout=1000)
            expect(current).to_have_count(0)
            pending_active[0].fetch()
            pending_active[0].abort("failed")
            page.unroute("**/api/forget")
            page.wait_for_timeout(2200)
            expect(reconnect).to_be_visible()
            page.unroute("**/api/status")
            page.route("**/api/status", lambda route: route.abort("failed"))
            expect(page.get_by_text("The device is currently unreachable.", exact=False)).to_be_visible(timeout=15000)
            expect(reconnect).to_be_visible()
            expect(page.get_by_role("heading", name="Connected to:", exact=False)).not_to_be_visible()
            page.screenshot(path=str(output / "forgot-current-network.png"), full_page=True)
            page.unroute("**/api/status")
            expect(page.get_by_role("heading", name="Connect to Wi-Fi", exact=True)).to_be_visible(timeout=15000)
            expect(page.get_by_role("button", name="Connect device")).to_be_enabled(timeout=15000)
            final_status = page.request.get("http://127.0.0.1:18080/api/status").json()
            assert final_status["link"] == "Disconnected"
            assert final_status["connected_ssid"] is None
            assert all(n["ssid"] != "Home Wi-Fi" for n in final_status["saved"])
            assert all("password" not in url for url in requested_urls), "secret entered a URL"
            assert not exceptions, exceptions
            assert not external, external
            browser.close()
        print("PASS: mobile onboarding, bad-password retry, lost response, connected management, offline assets, no browser secret storage")
    finally:
        server.terminate()
        server.wait(timeout=10)
        fixture.cleanup()


if __name__ == "__main__":
    main()
