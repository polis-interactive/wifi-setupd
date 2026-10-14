# wifi-setupd-zbus-api

Public version 1 contract for `tv.polis.WifiSetup1`, object
`/tv/polis/WifiSetup1`, interface `tv.polis.WifiSetup1`. The crate contains typed
async zbus proxies and public types, with no daemon, iwd, Kameo, or UI dependency.
It uses zbus's Tokio runtime; consumers supply their own connection and select
their Rust toolchain. The repository does not impose a rustup override.

## Properties and wire format

All properties are read-only and emit standard
`org.freedesktop.DBus.Properties.PropertiesChanged` notifications.
Enum values are case-sensitive strings matching the Rust variant names.

| Property | D-Bus signature | Values / meaning |
| --- | --- | --- |
| `State` | `s` | `Idle`, `StartingAccessPoint`, `AwaitingCredentials`, `Connecting`, `Verifying`, `Stopping`, `Failed` |
| `ProvisioningActive` | `b` | True except in `Idle` and `Failed`; includes cleanup in `Stopping` |
| `LinkState` | `s` | `Unknown`, `Disconnected`, `Associating`, `Associated`, `Ready` |
| `BackendState` | `s` | `Unavailable`, `Available` |
| `LastError` | `s` | `None`, `Cancelled`, `BackendUnavailable`, `AuthenticationFailed`, `NetworkUnavailable`, `TimedOut`, `AddressUnavailable`, `AccessPointFailed`, `StorageFailed`, `Internal` |
| `Status` | `(ssss)` | Coherent tuple `(State, LinkState, BackendState, LastError)` |

`Idle` means no active provisioning workflow, including an unconfigured or
backend-unavailable device. `Ready` requires authenticated association (where
applicable) and usable IP configuration on the Wi-Fi station interface. Neither
Ethernet nor Internet access can establish it. `Associated` alone is insufficient.
On backend loss, publish `Unavailable` with `Unknown` link readiness until fresh
observations establish the state. `Failed` is terminal, with no active portal;
a recovered portal is `AwaitingCredentials` and retains the attempt's error.

Read `Status` when multiple fields inform a decision: separate property reads
can observe different revisions. Subscriptions are notifications, not a durable
event history; slow consumers may see only the latest value. Reconnect, recreate
the proxy, and read a fresh `Status` after service/bus reconnection. Version 1's
strings and tuple layout are fixed. Unknown enum strings fail deserialization;
incompatible contract changes require a new interface version.

## Start, Stop, cancellation, and errors

Both methods take no arguments and return an empty successful reply. Success
means the command was accepted, not that the radio transition is complete.

| Request / current state | Required behavior |
| --- | --- |
| Start in `Idle` or `Failed` with `Ready` Wi-Fi | Idempotent success; preserve the working connection and keep the AP off |
| Start in `Idle` or `Failed` without `Ready` Wi-Fi | If backend available, accept a new workflow, clear LastError, publish `StartingAccessPoint` |
| Start during an active workflow except `Stopping` | Idempotent success; do not restart, clear LastError, or create another operation |
| Start in `Stopping` | Reject with `tv.polis.WifiSetup1.Error.Busy` |
| Start in `Idle` or `Failed` with unavailable backend | Reject with `tv.polis.WifiSetup1.Error.BackendUnavailable`; leave status unchanged |
| Stop during an active workflow | Accept cancellation, enter `Stopping`; repeated Stop is idempotent |
| Stop in `Idle` or `Failed` | Idempotent success; preserve LastError and existing Wi-Fi connection |

A stopped attempt must invalidate late results and reconcile accepted backend
actions before declaring cleanup complete or accepting a conflicting operation.
Stop remains acceptable during backend loss. While reconciliation is pending,
remain `Stopping`; do not claim the radio stopped merely because a future was
dropped. Successful cancellation ends in `Idle` with `Cancelled`. Failed cleanup
ends in `Failed` with a sanitized error only once conflicting operations are
quiescent; otherwise remain `Stopping`. Stop never forgets saved networks and
never emits `ProvisioningCompleted`.

`LastError` starts at `None`. An accepted new Start, an accepted new credential
attempt, or successful provisioning clears it. A failed accepted attempt sets
it; restoring the portal, repeated commands, and immediate rejections preserve
it. Cancellation sets `Cancelled` after cleanup. Error state is not persisted
across daemon restarts. Backend diagnostics and credentials never appear in this
property. Immediate command errors use `ControlError` and do not overwrite
LastError; `Internal` is reserved for an inability to accept the command, with a
sanitized message. Transport/standard D-Bus failures use the `Transport` variant.

After successful Wi-Fi verification, publish `Idle`, `Ready`, `Available`,
`None`, then emit the zero-argument `ProvisioningCompleted` signal exactly once
for that accepted attempt. The daemon stays callable. The signal is transient;
clients that missed it use current status. No signal is emitted for failure,
cancellation, or a repeated request.

## Rust client

```rust,no_run
use wifi_setupd_zbus_api::WifiSetupProxy;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let connection = zbus::Connection::system().await?;
let wifi = WifiSetupProxy::new(&connection).await?;
let updates = wifi.receive_status_changed().await;
let status = wifi.status().await?;
wifi.start_provisioning().await?;
# Ok(()) }
```

Use `futures_util::StreamExt::next` on `updates` and `.get().await` on each
property notification to obtain the latest typed snapshot. Subscribe before
issuing commands. `receive_provisioning_completed()` subscribes to completion.
See `examples/status.rs` for a runnable read-only consumer accepting an explicit
private-bus address. The daemon now exports the production actor/service; see
[`docs/provisioner.md`](../../docs/provisioner.md) for startup configuration.
The public D-Bus interface remains Start/Stop and status. The later HTTP portal
will use the daemon's internal credential-submission entry point.

The bindings follow the upstream [zbus proxy documentation](https://docs.rs/zbus/5.19.0/zbus/attr.proxy.html)
and [zvariant type documentation](https://docs.rs/zvariant/5.15.0/zvariant/derive.Type.html).
