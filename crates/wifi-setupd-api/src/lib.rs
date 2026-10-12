//! Public identifiers for the versioned wifi-setupd D-Bus service.
//!
//! Typed proxies, states, and errors will be added with the public API.
//! Internal iwd bindings belong in the daemon, not in this crate.

/// Well-known system bus name.
pub const BUS_NAME: &str = "tv.polis.WifiSetup1";

/// Object exporting the public provisioning interface.
pub const OBJECT_PATH: &str = "/tv/polis/WifiSetup1";

/// Versioned public interface name.
pub const INTERFACE_NAME: &str = "tv.polis.WifiSetup1";
