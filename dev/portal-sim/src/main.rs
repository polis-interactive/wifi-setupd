//! Development-only in-memory radio. Never touches D-Bus or host networking.
mod config;
use config::Config as Simulation;
use std::{path::PathBuf, time::Duration};
use wifi_setupd::{
    http,
    provisioner::{Config, Handle},
    wifi::*,
};
use wifi_setupd_zbus_api::{BackendState, LinkState};
struct Radio {
    snapshot: Snapshot,
    fixtures: Vec<config::Network>,
    saved: Vec<Credentials>,
    connected: Option<Network>,
}
fn network(ssid: &str, password: &str) -> Network {
    Network {
        ssid: ssid.into(),
        security: if password.is_empty() {
            Security::Open
        } else {
            Security::Personal
        },
    }
}
impl Radio {
    fn new(config: Simulation) -> Self {
        let connected = config
            .initially_connected
            .as_ref()
            .and_then(|ssid| config.networks.iter().find(|n| n.ssid == *ssid))
            .map(|n| network(&n.ssid, &n.password));
        let saved = config
            .networks
            .iter()
            .filter(|n| n.saved || config.initially_connected.as_ref() == Some(&n.ssid))
            .map(|n| Credentials {
                ssid: n.ssid.clone(),
                password: n.password.clone(),
            })
            .collect();
        Self {
            snapshot: Snapshot {
                backend: BackendState::Available,
                link: if connected.is_some() {
                    LinkState::Ready
                } else {
                    LinkState::Disconnected
                },
                access_point_active: false,
                connected_ssid: None,
            },
            fixtures: config.networks,
            saved,
            connected,
        }
    }
    async fn connect(&mut self, c: Credentials) -> Result<(), Error> {
        self.snapshot.link = LinkState::Associating;
        let fixture = self
            .fixtures
            .iter()
            .find(|n| n.ssid == c.ssid && n.visible)
            .ok_or(Error::NetworkUnavailable)?;
        tokio::time::sleep(Duration::from_millis(fixture.connect_delay_ms)).await;
        if fixture.password != c.password {
            return Err(Error::Authentication);
        }
        self.connected = Some(network(&c.ssid, &c.password));
        self.saved.retain(|n| n.ssid != c.ssid);
        self.saved.push(c);
        self.snapshot.link = LinkState::Ready;
        Ok(())
    }
}
impl Adapter for Radio {
    async fn snapshot(&mut self) -> Result<Snapshot, Error> {
        let mut snapshot = self.snapshot.clone();
        snapshot.connected_ssid = self.connected.as_ref().map(|network| network.ssid.clone());
        Ok(snapshot)
    }
    async fn scan(&mut self) -> Result<Vec<Network>, Error> {
        Ok(self
            .fixtures
            .iter()
            .filter(|n| n.visible)
            .map(|n| network(&n.ssid, &n.password))
            .collect())
    }
    async fn known_networks(&mut self) -> Result<Vec<KnownNetwork>, Error> {
        Ok(self
            .saved
            .iter()
            .map(|n| KnownNetwork {
                network: network(&n.ssid, &n.password),
                auto_connect: true,
            })
            .collect())
    }
    async fn apply(&mut self, _: OperationId, action: Action) -> Result<Snapshot, Error> {
        match action {
            Action::StartAccessPoint => {
                self.snapshot.access_point_active = true;
                self.snapshot.link = LinkState::Disconnected;
                self.connected = None;
            }
            Action::StopAccessPoint => self.snapshot.access_point_active = false,
            Action::Connect(c) => {
                if self.saved.iter().any(|n| n.ssid == c.ssid) {
                    return Err(Error::CredentialConflict);
                }
                self.connect(c).await?;
            }
            Action::ReplaceCredentials(c) => {
                self.saved.retain(|n| n.ssid != c.ssid);
                self.connect(c).await?;
            }
            Action::ConnectKnown(selected) => {
                let saved = self
                    .saved
                    .iter()
                    .find(|n| network(&n.ssid, &n.password) == selected)
                    .ok_or(Error::NetworkUnavailable)?;
                self.connect(Credentials {
                    ssid: saved.ssid.clone(),
                    password: saved.password.clone(),
                })
                .await?;
            }
            Action::Disconnect => {
                self.snapshot.link = LinkState::Disconnected;
                self.connected = None;
            }
            Action::Forget { network: selected } => {
                self.saved
                    .retain(|n| network(&n.ssid, &n.password) != selected);
                if self.connected.as_ref() == Some(&selected) {
                    self.connected = None;
                    self.snapshot.link = LinkState::Disconnected;
                }
            }
        }
        self.snapshot().await
    }
    async fn cancel_and_reconcile(&mut self, _: OperationId) -> Result<Snapshot, Error> {
        self.snapshot.link = LinkState::Disconnected;
        self.snapshot.access_point_active = false;
        self.connected = None;
        self.snapshot().await
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or("usage: portal-sim WEB_ROOT [CONFIG.toml] [--dev]")?,
    );
    let input = match args.next() {
        Some(path) => std::fs::read_to_string(path)?,
        None => include_str!("../example.toml").into(),
    };
    let development = match args.next().as_deref() {
        None => false,
        Some("--dev") => true,
        _ => return Err("expected --dev after CONFIG.toml".into()),
    };
    if args.next().is_some() {
        return Err("usage: portal-sim WEB_ROOT [CONFIG.toml] [--dev]".into());
    }
    if !development && !root.join("index.html").is_file() {
        return Err("Build the UI first; WEB_ROOT must contain index.html".into());
    }
    let simulation = Simulation::parse(&input)?;
    if development && simulation.port == 8089 {
        return Err("port 8089 is reserved for the development API".into());
    }
    let port = if development { 8089 } else { simulation.port };
    let address = format!("127.0.0.1:{port}");
    let frontend_origin = if simulation.port == 80 {
        "http://127.0.0.1".into()
    } else {
        format!("http://127.0.0.1:{}", simulation.port)
    };
    let authority = if port == 80 {
        "127.0.0.1".into()
    } else {
        address.clone()
    };
    let http_config = http::Config {
        web_root: root,
        ap_name: simulation.ap_name.clone(),
        device_key: Some(simulation.device_key.clone()),
        hostnames: vec![],
    };
    let handle = Handle::spawn(
        Radio::new(simulation),
        Config {
            startup_grace: Duration::ZERO,
            ..Config::default()
        },
    )?;
    let router = http::router(handle, http_config, authority)?;
    // dx rewrites Host to the upstream but preserves the browser's Origin.
    // Adapt only the explicit local development origin before production checks.
    let router = if development {
        router.layer(axum::middleware::from_fn(
            move |mut request: axum::extract::Request, next: axum::middleware::Next| {
                let frontend_origin = frontend_origin.clone();
                async move {
                    if request
                        .headers()
                        .get("origin")
                        .and_then(|v| v.to_str().ok())
                        == Some(&frontend_origin)
                    {
                        request
                            .headers_mut()
                            .insert("origin", "http://127.0.0.1:8089".parse().unwrap());
                    }
                    next.run(request).await
                }
            },
        ))
    } else {
        router
    };
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!(
        "Portal simulator: http://{address} (TOML fixtures; Ctrl+C stops; restart resets saved networks)"
    );
    axum::serve(listener, router).await?;
    Ok(())
}
#[cfg(test)]
mod tests;
