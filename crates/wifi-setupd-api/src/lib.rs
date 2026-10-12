//! Version 1 of the public provisioning contract. See the crate README for
//! wire values and command semantics. No iwd or actor types cross this boundary.
//!
//! ```no_run
//! # async fn example(connection: &zbus::Connection) -> Result<(), Box<dyn std::error::Error>> {
//! use wifi_setupd_api::WifiSetupProxy;
//! let wifi = WifiSetupProxy::new(connection).await?;
//! let status = wifi.status().await?;
//! println!("{:?}: {:?}", status.state, status.link);
//! wifi.start_provisioning().await?;
//! # Ok(()) }
//! ```

use serde::{Deserialize, Serialize};
use zbus::zvariant::{OwnedValue, Type, Value};

pub const BUS_NAME: &str = "tv.polis.WifiSetup1";
pub const OBJECT_PATH: &str = "/tv/polis/WifiSetup1";
pub const INTERFACE_NAME: &str = "tv.polis.WifiSetup1";

/// Stable, case-sensitive D-Bus strings; discriminant numbers are not wire values.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue,
)]
#[zvariant(signature = "s", crate = "zbus::zvariant")]
pub enum ProvisioningState {
    #[default]
    Idle,
    StartingAccessPoint,
    AwaitingCredentials,
    Connecting,
    Verifying,
    Stopping,
    Failed,
}

impl ProvisioningState {
    /// Includes cancellation cleanup. Failed is terminal and has no active portal.
    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Idle | Self::Failed)
    }
}

/// Readiness of the Wi-Fi station interface, never Ethernet or Internet reachability.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue,
)]
#[zvariant(signature = "s", crate = "zbus::zvariant")]
pub enum LinkState {
    #[default]
    Unknown,
    Disconnected,
    Associating,
    Associated,
    Ready,
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue,
)]
#[zvariant(signature = "s", crate = "zbus::zvariant")]
pub enum BackendState {
    #[default]
    Unavailable,
    Available,
}

/// Sanitized outcome of the last accepted provisioning attempt. No backend text,
/// network identifiers, credentials, or object paths are exposed here.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue,
)]
#[zvariant(signature = "s", crate = "zbus::zvariant")]
pub enum ProvisioningError {
    #[default]
    None,
    Cancelled,
    BackendUnavailable,
    AuthenticationFailed,
    NetworkUnavailable,
    TimedOut,
    AddressUnavailable,
    AccessPointFailed,
    StorageFailed,
    Internal,
}

/// Coherent snapshot with D-Bus signature `(ssss)`, in this field order.
/// Individual properties may span different revisions; use Status for decisions.
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue,
)]
#[zvariant(crate = "zbus::zvariant")]
pub struct Status {
    pub state: ProvisioningState,
    pub link: LinkState,
    pub backend: BackendState,
    pub last_error: ProvisioningError,
}

impl Status {
    pub const fn provisioning_active(self) -> bool {
        self.state.is_active()
    }
}

/// Immediate command rejection, distinct from an asynchronous LastError.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "tv.polis.WifiSetup1.Error")]
pub enum ControlError {
    Busy(String),
    BackendUnavailable(String),
    Internal(String),
    #[zbus(error)]
    Transport(zbus::Error),
}

/// Commands acknowledge acceptance, not completion. Callers supply the connection:
/// production uses the system bus; tests use an explicitly addressed private bus.
#[zbus::proxy(
    interface = "tv.polis.WifiSetup1",
    default_service = "tv.polis.WifiSetup1",
    default_path = "/tv/polis/WifiSetup1",
    gen_blocking = false
)]
pub trait WifiSetup {
    fn start_provisioning(&self) -> Result<(), ControlError>;
    fn stop_provisioning(&self) -> Result<(), ControlError>;

    #[zbus(property)]
    fn state(&self) -> zbus::Result<ProvisioningState>;
    #[zbus(property)]
    fn provisioning_active(&self) -> zbus::Result<bool>;
    #[zbus(property)]
    fn last_error(&self) -> zbus::Result<ProvisioningError>;
    #[zbus(property)]
    fn link_state(&self) -> zbus::Result<LinkState>;
    #[zbus(property)]
    fn backend_state(&self) -> zbus::Result<BackendState>;
    #[zbus(property)]
    fn status(&self) -> zbus::Result<Status>;

    /// Emitted once after successful association and usable Wi-Fi IP configuration.
    /// Properties are updated first. This transient signal is not replayed.
    #[zbus(signal)]
    fn provisioning_completed(&self) -> zbus::Result<()>;
}
