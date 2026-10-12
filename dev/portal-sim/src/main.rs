//! Development-only in-memory radio. Never touches D-Bus or host networking.
use std::time::Duration;
use wifi_setupd::{
    http,
    provisioner::{Config, Handle},
    wifi::*,
};
use wifi_setupd_zbus_api::{BackendState, LinkState};
struct Radio {
    snapshot: Snapshot,
    saved: Vec<Network>,
}
fn network(ssid: &str) -> Network {
    Network {
        ssid: ssid.into(),
        security: Security::Personal,
    }
}
impl Adapter for Radio {
    async fn snapshot(&mut self) -> Result<Snapshot, Error> {
        Ok(self.snapshot.clone())
    }
    async fn scan(&mut self) -> Result<Vec<Network>, Error> {
        Ok(vec![network("Home Wi-Fi"), network("Workshop")])
    }
    async fn known_networks(&mut self) -> Result<Vec<KnownNetwork>, Error> {
        Ok(self
            .saved
            .iter()
            .cloned()
            .map(|network| KnownNetwork {
                network,
                auto_connect: true,
            })
            .collect())
    }
    async fn apply(&mut self, _: OperationId, action: Action) -> Result<Snapshot, Error> {
        match action {
            Action::StartAccessPoint => {
                self.snapshot.access_point_active = true;
                self.snapshot.link = LinkState::Disconnected;
            }
            Action::StopAccessPoint => self.snapshot.access_point_active = false,
            Action::Connect(c) | Action::ReplaceCredentials(c) => {
                self.snapshot.link = LinkState::Associating;
                tokio::time::sleep(Duration::from_secs(2)).await;
                if c.password == "wrong-password" {
                    return Err(Error::Authentication);
                }
                self.saved.retain(|n| n.ssid != c.ssid);
                self.saved.push(network(&c.ssid));
                self.snapshot.link = LinkState::Ready;
            }
            Action::ConnectKnown(_) => self.snapshot.link = LinkState::Ready,
            Action::Disconnect => self.snapshot.link = LinkState::Disconnected,
            Action::Forget { network } => self.saved.retain(|n| n != &network),
        }
        Ok(self.snapshot.clone())
    }
    async fn cancel_and_reconcile(&mut self, _: OperationId) -> Result<Snapshot, Error> {
        self.snapshot.link = LinkState::Disconnected;
        self.snapshot.access_point_active = false;
        Ok(self.snapshot.clone())
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args()
        .nth(1)
        .ok_or("usage: portal-sim WEB_ROOT")?;
    let handle = Handle::spawn(
        Radio {
            snapshot: Snapshot {
                backend: BackendState::Available,
                link: LinkState::Disconnected,
                access_point_active: false,
            },
            saved: vec![network("Old network")],
        },
        Config {
            startup_grace: Duration::ZERO,
            ..Config::default()
        },
    )?;
    let router = http::router(
        handle,
        http::Config {
            web_root: root.into(),
            ap_name: "wifi-setupd-test".into(),
            management_key: Some("simulator-management-key-for-testing".into()),
        },
        "127.0.0.1:8080".into(),
    )?;
    println!("Portal simulator: http://127.0.0.1:8080 (wrong-password simulates rejection)");
    axum::serve(
        tokio::net::TcpListener::bind("127.0.0.1:8080").await?,
        router,
    )
    .await?;
    Ok(())
}
