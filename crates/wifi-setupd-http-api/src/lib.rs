//! HTTP data only: no daemon, D-Bus, or browser runtime dependencies.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    pub ssid: String,
    pub security: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortalStatus {
    pub state: String,
    pub link: String,
    pub backend: String,
    pub error: Option<String>,
    pub ap_name: String,
    pub csrf: String,
    pub networks: Vec<Network>,
    pub saved: Vec<Network>,
    pub saved_available: bool,
    pub scan_cached: bool,
    pub management_requires_key: bool,
}
// Never derive Debug for request bodies containing secrets.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    pub ssid: String,
    pub password: String,
    pub replace: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Accepted {
    pub attempt: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}
