---
name: dioxus-web
description: Implement and validate the wifi-setupd Dioxus 0.7 web frontend, including components, browser HTTP requests, routing, and static assets. Use for UI work in this repository, not daemon internals or hardware testing.
---

# Dioxus web development

Start with the official [Dioxus 0.7 documentation index](https://dioxuslabs.com/learn/0.7/llms.txt).
Use the linked topic documentation when implementing a feature; keep examples
and installed crate/CLI versions compatible with the selected 0.7 release.

## Components and browser work

Use `rsx!` components and signals for local UI state. Read the index's component
and state links before changing those APIs. Follow
[async and futures](https://dioxuslabs.com/learn/0.7/essentials/basics/async/)
and [data fetching](https://dioxuslabs.com/learn/0.7/essentials/basics/resources/)
for asynchronous requests and loading/error states.

Use browser-compatible HTTP calls to relative daemon endpoints. Handle duplicate
submits, unmounts, and lost connectivity; a lost HTTP response does not prove a
provisioning operation failed. Keep passwords out of URLs and browser storage.
Use the web renderer; the application does not need desktop or fullstack servers.

## Assets and routing

Follow [assets](https://dioxuslabs.com/learn/0.7/essentials/ui/assets/)
for package-relative `asset!` references and CLI processing. Asset metadata does
not itself embed files into the daemon; packaging the generated web output is
a separate build step. Serve all needed assets locally for offline onboarding.

Use a single screen initially. If routing becomes useful, read
[routing](https://dioxuslabs.com/learn/0.7/essentials/router/)
and verify direct navigation with Axum's static serving behavior.

## Validation

Verify the selected CLI's web build command and build actual WASM/static output;
a native Cargo check alone does not validate the browser app. Check the generated
app in a mobile-sized browser against the simulated HTTP backend, including
failure, retry, and the expected disconnect during an AP-to-station transition.
Keep general architecture and hardware procedures in `docs/`.
