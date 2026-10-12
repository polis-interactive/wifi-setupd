Implement a standalone Rust Wi-Fi provisioning daemon called **`wifi-setupd`** in this empty repository.

The target is a minimal Buildroot-based embedded Linux appliance, primarily Raspberry Pi, running BusyBox init.

The application should provide reliable Wi-Fi onboarding through a captive portal, with a reusable D-Bus API that other appliance applications can consume.

Prioritize reliability, maintainability, and a small runtime footprint. This is an embedded appliance service, not a general-purpose network manager.

### 1. Technology stack

Use:

- **Rust + Tokio** — Async runtime.
- **Kameo** — Actor-based state ownership, messaging, and supervision.
- **zbus** — System D-Bus communication.
- **iwd** — Wi-Fi authentication, connection management, AP mode, and DHCP.
- **iwdrs** — Evaluate for iwd integration; isolate behind an adapter so it can be replaced with direct zbus proxies if necessary.
- **Axum** — HTTP endpoints and captive-portal backend.
- **Dioxus** — Web interface.
- **Serde** — Configuration and serialization.
- **Tracing** — Structured logging and diagnostics.

Do not depend on NetworkManager, ConnMan, systemd, Docker, or external cloud services.

Use current stable crate versions compatible with the selected Rust toolchain.

### 2. Workspace structure

Create a Cargo workspace containing three crates:

```text
wifi-setupd/
├── Cargo.toml
├── crates/
│   ├── wifi-setupd/
│   │   └── src/
│   │       ├── main.rs
│   │       ├── wifi/
│   │       ├── provisioning/
│   │       ├── dbus/
│   │       └── http/
│   │
│   ├── wifi-setupd-api/
│   │   └── src/
│   │       └── lib.rs
│   │
│   └── wifi-setupd-ui/
│       └── src/
│           └── main.rs
│
└── README.md
```

**`wifi-setupd`**

The Linux daemon. Owns provisioning state, communicates with iwd, runs the captive portal backend, and exposes its public D-Bus interface.

**`wifi-setupd-api`**

A reusable Rust library containing the daemon's public D-Bus contract.

**Important:** This library exposes the D-Bus interfaces implemented by `wifi-setupd`, NOT the internal bindings used to communicate with iwd.

Other applications should depend on this crate without needing to understand or depend on iwd, Kameo, or the daemon's internal implementation.

**`wifi-setupd-ui`**

A Dioxus web application compiled to static assets/WASM and served by the daemon.

Keep the UI independent of the daemon's internal state management. It communicates through HTTP endpoints.

### 3. D-Bus interface

Use the Polis namespace, derived from `polis.tv`.

```text
Service:    tv.polis.WifiSetup1
Object:     /tv/polis/WifiSetup1
Interface:  tv.polis.WifiSetup1
```

Expose an initial interface resembling:

```text
Properties:
    State
    ProvisioningActive
    LastError

Methods:
    StartProvisioning()
    StopProvisioning()

Signals:
    ProvisioningCompleted()
```

Use standard D-Bus `PropertiesChanged` notifications for state updates where appropriate.

The API crate should provide:

- Public types representing provisioning states and errors.
- Typed zbus proxy bindings.
- Constants defining bus name, object path, and interface.
- Convenient methods for querying status and controlling provisioning.
- Subscriptions to property changes.
- Documentation and examples demonstrating integration into another Rust application.

Ideally, consumers can use:

```rust
use wifi_setupd_api::WifiSetupProxy;

let connection = zbus::Connection::system().await?;
let wifi = WifiSetupProxy::new(&connection).await?;

let state = wifi.state().await?;
wifi.start_provisioning().await?;
```

The exact signatures can differ if necessary.

The daemon must remain running after provisioning completes so other applications can request provisioning later.

Keep the public API versioned and independent from implementation details.

### 4. iwd integration

Use iwd as the authoritative source of Wi-Fi connectivity.

Initially evaluate [iwdrs](https://github.com/pythops/iwdrs) for communicating with iwd.

Known concerns include incomplete API coverage, error handling, and D-Bus object lifecycle management.

Do not fork iwdrs immediately. Encapsulate it behind an internal interface so we can replace problematic functionality with direct zbus proxies.

Implement support for:

- Wi-Fi device discovery.
- Network scanning.
- WPA2/WPA3 authentication.
- Connecting to networks.
- Connection status and failures.
- Saved network management.
- Switching between station and AP modes.
- Starting/stopping a provisioning hotspot.
- Handling iwd restarts and D-Bus interruptions.

Use iwd's built-in network configuration and DHCP functionality where practical.

Avoid introducing additional daemons unless necessary.

### 5. Kameo state management

Use a `Provisioner` actor as the primary owner of provisioning state.

Start with a simple state machine:

```rust
enum ProvisioningState {
    Idle,
    StartingAccessPoint,
    AwaitingCredentials,
    Connecting,
    Verifying,
    Failed,
}
```

Use typed actor messages for commands and events:

```rust
struct StartProvisioning;
struct StopProvisioning;

struct CredentialsSubmitted {
    ssid: String,
    password: String,
}

struct ConnectionResult {
    success: bool,
}

struct ConnectionLost;
```

These are illustrative; refine the types as necessary.

Requirements:

- The actor exclusively owns mutable provisioning state.
- HTTP and D-Bus handlers communicate with the actor through messages.
- Long-running network operations must not block the actor mailbox.
- Use timeouts and cancellation where appropriate.
- Use operation IDs or equivalent mechanisms to reject stale asynchronous results.
- Publish read-only status snapshots using `tokio::sync::watch` where useful.
- Keep actor supervision simple.
- Reconstruct state from iwd and persistent configuration after a restart.

Do not create unnecessary actors.

Kameo should coordinate application behavior, not duplicate iwd's internal networking state.

### 6. Provisioning workflow

Implement the following behavior:

```text
BOOT
 |
 +-- Existing Wi-Fi configuration?
 |          |
 |          +-- Yes --> Attempt connection
 |          |               |
 |          |               +-- Success --> Idle
 |          |               |
 |          |               +-- Failure --> Retry / Recovery
 |          |
 |          +-- No --> Start provisioning
 |
 +-- D-Bus StartProvisioning()
            |
            v
      Start Wi-Fi AP
            |
      Start HTTP portal
            |
      Await credentials
            |
      Stop AP / Switch to station
            |
      Attempt connection
            |
        +---+---+
        |       |
      Success  Failure
        |       |
       Idle   Restore AP
```

The provisioning implementation must account for single-radio hardware that cannot reliably operate in AP and station modes simultaneously.

The UI should clearly communicate that the user's phone may disconnect when the device switches to the target network.

Reliability requirements:

- Preserve previously working credentials until replacements are verified.
- Recover from incorrect credentials.
- Handle unavailable networks and authentication failures.
- Apply connection timeouts.
- Handle power loss and daemon restarts.
- Avoid overlapping provisioning attempts.
- Avoid aggressive AP/client switching during transient connectivity loss.
- Support manual provisioning through D-Bus.
- Do not require Internet access to consider Wi-Fi provisioning successful.
- Persist configuration on a configurable writable filesystem location.
- Never expose credentials through logs, D-Bus properties, or signals.

Design transitions to be deterministic and testable.

### 7. Captive portal

Use Axum for the backend and Dioxus for the frontend.

The captive portal should:

- Display available Wi-Fi networks.
- Accept credentials.
- Display connection progress.
- Report errors and offer retries.
- Work on mobile browsers.
- Support direct navigation to a known local IP address.

Use iwd's integrated AP/DHCP functionality where possible.

Add DNS interception only if required for captive-portal behavior.

The Dioxus app should compile to static assets/WASM hosted by the daemon. It must not require Node.js, a browser engine, or a separate application server on the appliance.

Support bundling UI assets into the daemon for simple deployment.

**Dioxus documentation/skill:** Use the official Dioxus 0.7 documentation at:

https://dioxuslabs.com/learn/0.7/llms.txt

Read it and use it as the foundation for a repository-local Codex skill covering Dioxus development. Follow the documentation links when implementing components, routing, assets, and backend communication. Keep the skill focused on Dioxus rather than duplicating the project's general architecture documentation.

Keep the initial UI functional and minimal. Avoid spending significant effort on styling.

### 8. Buildroot compatibility

The resulting application must:

- Work with BusyBox init.
- Use the system D-Bus.
- Work without systemd.
- Support ARM Linux cross-compilation.
- Run with musl or glibc where dependencies permit.
- Avoid desktop-environment dependencies.
- Operate on an isolated network without Internet access.
- Use persistent storage only where necessary.

Provide documentation covering:

- Required Buildroot packages.
- iwd configuration.
- D-Bus system policy.
- Service startup order.
- Filesystem paths and permissions.
- Building and installing the daemon and UI.

Do not assume that the production root filesystem is writable.

### 9. Testing

Separate provisioning logic from hardware-specific operations.

Provide mockable interfaces for iwd operations.

Test at least:

- First boot without credentials.
- Successful provisioning.
- Incorrect password.
- Unavailable network.
- Connection timeout.
- Repeated provisioning requests.
- Interrupted AP/station transitions.
- Restart during provisioning.
- Recovery after iwd restarts.
- Credential preservation following failure.
- D-Bus client reconnection and state synchronization.

Tests should run without requiring actual Wi-Fi hardware wherever possible.

### 10. Implementation approach

Build the system incrementally:

1. Initialize the Cargo workspace and shared API crate.
2. Implement the iwd adapter and validate basic wireless operations.
3. Implement the Kameo provisioning state machine.
4. Implement the public D-Bus service and client bindings.
5. Implement AP/station transitions and provisioning recovery.
6. Implement the captive portal backend.
7. Add the Dioxus frontend.
8. Integrate frontend assets into the daemon.
9. Add tests and Buildroot integration documentation.

Ensure each component compiles and can be tested independently.

Prefer working implementations over elaborate scaffolding. Avoid unnecessary abstraction, excessive configuration, or premature generalization.

### 11. Design principles

The project should remain a small, reusable component of a larger embedded Linux platform.

Specifically:

- **iwd owns Wi-Fi connectivity.**
- **Kameo owns application state and workflows.**
- **zbus provides interprocess communication.**
- **`wifi-setupd-api` defines our public contract.**
- **Axum exposes the provisioning HTTP API.**
- **Dioxus provides the web interface.**
- **Tokio provides asynchronous execution.**

The public D-Bus API should be the primary integration point for other appliance applications.

Do not couple the implementation to any particular lighting application or other Polis product.

Start implementing in the existing empty repository. Make reasonable technical decisions independently, document meaningful tradeoffs, and prioritize a working end-to-end provisioning flow over secondary features.