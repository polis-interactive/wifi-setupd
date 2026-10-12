//! Private bindings checked against the installed iwd 3.12 source interfaces.
use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

pub const SERVICE: &str = "net.connman.iwd";
pub const MANAGER: &str = "/net/connman/iwd";
pub const DEVICE: &str = "net.connman.iwd.Device";
pub const STATION: &str = "net.connman.iwd.Station";
pub const NETWORK: &str = "net.connman.iwd.Network";
pub const KNOWN: &str = "net.connman.iwd.KnownNetwork";
pub const AP: &str = "net.connman.iwd.AccessPoint";

#[zbus::proxy(interface = "net.connman.iwd.Device", gen_blocking = false)]
pub trait Device {
    #[zbus(property)]
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "net.connman.iwd.Station", gen_blocking = false)]
pub trait Station {
    fn scan(&self) -> zbus::Result<()>;
    fn get_ordered_networks(&self) -> zbus::Result<Vec<(OwnedObjectPath, i16)>>;
    fn disconnect(&self) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "net.connman.iwd.Network", gen_blocking = false)]
pub trait Network {
    fn connect(&self) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "net.connman.iwd.KnownNetwork", gen_blocking = false)]
pub trait KnownNetwork {
    fn forget(&self) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "net.connman.iwd.AgentManager", gen_blocking = false)]
pub trait AgentManager {
    fn register_agent(&self, path: &ObjectPath<'_>) -> zbus::Result<()>;
    fn unregister_agent(&self, path: &ObjectPath<'_>) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "net.connman.iwd.AccessPoint", gen_blocking = false)]
pub trait AccessPoint {
    fn start(&self, ssid: &str, passphrase: &str) -> zbus::Result<()>;
    fn start_profile(&self, ssid: &str) -> zbus::Result<()>;
    fn stop(&self) -> zbus::Result<()>;
}

#[zbus::proxy(
    interface = "net.connman.iwd.AccessPointDiagnostic",
    gen_blocking = false
)]
pub trait AccessPointDiagnostic {
    fn get_diagnostics(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}
