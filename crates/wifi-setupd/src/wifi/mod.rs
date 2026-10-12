//! Backend boundary. Connections and interface selection are explicit. iwd is
//! the sole credential store; the daemon never reads or writes its profiles.
pub mod iwd;
use std::{fmt, future::Future};
use wifi_setupd_zbus_api::{BackendState, LinkState, ProvisioningError};

/// Temporary input for a new connection, not a credential record. An empty
/// password selects an open network. Never derive Debug for secrets.
#[derive(PartialEq, Eq)]
pub struct Credentials {
    pub ssid: String,
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials([redacted])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: String,
    pub security: Security,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownNetwork {
    pub network: Network,
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    Personal,
    Enterprise,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub backend: BackendState,
    pub link: LinkState,
    pub access_point_active: bool,
}

/// IDs are assigned by the provisioner, never derived from an iwd object path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationId(pub u64);

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    StartAccessPoint,
    StopAccessPoint,
    Connect(Credentials),
    /// Explicit user intent: discard this network's old profile through iwd,
    /// then submit the new credentials. Failure does not restore the old secret.
    ReplaceCredentials(Credentials),
    /// Use iwd's existing profile without reading or replacing its secrets.
    ConnectKnown(Network),
    Disconnect,
    Forget {
        network: Network,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Unavailable,
    Authentication,
    NetworkUnavailable,
    Timeout,
    AddressUnavailable,
    AccessPoint,
    Storage,
    Cancelled,
    Internal,
    Busy,
    Unsupported,
    InvalidInput,
    /// Existing profile requires ConnectKnown or explicit ReplaceCredentials.
    CredentialConflict,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}

impl From<Error> for ProvisioningError {
    fn from(error: Error) -> Self {
        match error {
            Error::Unavailable => Self::BackendUnavailable,
            Error::Authentication => Self::AuthenticationFailed,
            Error::NetworkUnavailable => Self::NetworkUnavailable,
            Error::Timeout => Self::TimedOut,
            Error::AddressUnavailable => Self::AddressUnavailable,
            Error::AccessPoint => Self::AccessPointFailed,
            Error::Storage => Self::StorageFailed,
            Error::Cancelled => Self::Cancelled,
            Error::Internal
            | Error::Busy
            | Error::Unsupported
            | Error::InvalidInput
            | Error::CredentialConflict => Self::Internal,
        }
    }
}

/// Successful `apply` returns observed state, not just request acceptance.
/// Association alone is not Ready: the adapter must verify a usable Wi-Fi IP.
/// Futures may be dropped, but accepted backend actions can continue. Before a
/// conflicting operation, call cancel_and_reconcile and use its fresh snapshot.
/// Backend reconnection/object replacement also requires a fresh snapshot.
pub trait Adapter: Send {
    fn snapshot(&mut self) -> impl Future<Output = Result<Snapshot, Error>> + Send;
    fn scan(&mut self) -> impl Future<Output = Result<Vec<Network>, Error>> + Send;
    fn known_networks(&mut self) -> impl Future<Output = Result<Vec<KnownNetwork>, Error>> + Send;
    fn apply(
        &mut self,
        operation: OperationId,
        action: Action,
    ) -> impl Future<Output = Result<Snapshot, Error>> + Send;
    /// Do not report success until the old action is quiescent and actual device
    /// state is known. An error forbids assuming cleanup has completed.
    fn cancel_and_reconcile(
        &mut self,
        operation: OperationId,
    ) -> impl Future<Output = Result<Snapshot, Error>> + Send;
}

#[cfg(test)]
mod tests;
