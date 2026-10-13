use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub port: u16,
    pub ap_name: String,
    pub device_key: String,
    pub initially_connected: Option<String>,
    pub networks: Vec<Network>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            port: 8080,
            ap_name: "wifi-setupd-test".into(),
            device_key: "simulator-device-password".into(),
            initially_connected: None,
            networks: Vec::new(),
        }
    }
}
// Fake passwords are fixtures, but avoid accidentally printing them in errors.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub ssid: String,
    pub password: String,
    #[serde(default = "connect_delay")]
    pub connect_delay_ms: u64,
    #[serde(default)]
    pub saved: bool,
    #[serde(default = "visible")]
    pub visible: bool,
}
fn connect_delay() -> u64 {
    2000
}
fn visible() -> bool {
    true
}
impl Config {
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        let config: Self = toml::from_str(input)
            .map_err(|_| "Invalid simulator TOML: check syntax and field names.")?;
        if config.port == 0 {
            return Err("Use a nonzero port.");
        }
        if !(8..=63).contains(&config.device_key.len())
            || !config.device_key.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err("device_key must be 8–63 printable ASCII characters without spaces.");
        }
        let mut names = HashSet::new();
        for network in &config.networks {
            if network.connect_delay_ms > 60_000 {
                return Err("Each network must have connect_delay_ms <= 60000.");
            }
            if network.ssid.is_empty()
                || network.ssid.len() > 32
                || network.ssid.contains('\0')
                || !names.insert(&network.ssid)
            {
                return Err("Network SSIDs must be unique and 1–32 bytes, without NUL characters.");
            }
            if !network.password.is_empty()
                && (!(8..=63).contains(&network.password.len())
                    || !network.password.is_ascii()
                    || network.password.contains('\0'))
            {
                return Err(
                    "Fake passwords must be 8–63 ASCII characters without NUL, or empty for open Wi-Fi.",
                );
            }
        }
        if config
            .initially_connected
            .as_ref()
            .is_some_and(|ssid| !config.networks.iter().any(|n| n.ssid == *ssid && n.visible))
        {
            return Err("initially_connected must name a visible configured network.");
        }
        Ok(config)
    }
}
