use super::*;
use crate::test_bus;
use std::sync::{Arc, Mutex, atomic::AtomicBool};
use test_bus::PrivateBus;
use zbus::{message::Header, object_server::ObjectServer};

const DEVICE_PATH: &str = "/net/connman/iwd/0/3";
const NETWORK_PATH: &str = "/net/connman/iwd/0/3/test_psk";
const KNOWN_PATH: &str = "/net/connman/iwd/known/test_psk";
const OTHER_PATH: &str = "/net/connman/iwd/known/other_psk";

struct State {
    mode: String,
    station: String,
    ap: bool,
    agent: Option<(String, OwnedObjectPath)>,
    connects: usize,
    mutations: usize,
    reject: bool,
    stall: bool,
    generation: usize,
    slow_properties: bool,
    slow_mode: bool,
}
impl Default for State {
    fn default() -> Self {
        Self {
            mode: "station".into(),
            station: "disconnected".into(),
            ap: false,
            agent: None,
            connects: 0,
            mutations: 0,
            reject: false,
            stall: false,
            generation: 0,
            slow_properties: false,
            slow_mode: false,
        }
    }
}
type Shared = Arc<Mutex<State>>;
struct Device(Shared);
#[zbus::interface(name = "net.connman.iwd.Device")]
impl Device {
    #[zbus(property)]
    async fn name(&self) -> &str {
        let slow = self.0.lock().unwrap().slow_properties;
        if slow {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        "test-wlan"
    }
    #[zbus(property)]
    fn powered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn mode(&self) -> String {
        self.0.lock().unwrap().mode.clone()
    }
    #[zbus(property)]
    async fn set_mode(&self, mode: &str) {
        let slow = self.0.lock().unwrap().slow_mode;
        if slow && mode == "ap" {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let mut state = self.0.lock().unwrap();
        state.mode = mode.into();
        state.station = "disconnected".into();
        state.mutations += 1;
    }
}
struct Station(Shared);
#[zbus::interface(name = "net.connman.iwd.Station")]
impl Station {
    fn scan(&self) {
        self.0.lock().unwrap().mutations += 1;
    }
    fn get_ordered_networks(&self) -> Vec<(OwnedObjectPath, i16)> {
        vec![(NETWORK_PATH.try_into().unwrap(), -4200)]
    }
    fn disconnect(&self) {
        let mut state = self.0.lock().unwrap();
        state.station = "disconnected".into();
        state.generation += 1;
        state.mutations += 1;
    }
    #[zbus(property)]
    fn state(&self) -> String {
        self.0.lock().unwrap().station.clone()
    }
    #[zbus(property)]
    fn scanning(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn connected_network(&self) -> zbus::fdo::Result<OwnedObjectPath> {
        if self.0.lock().unwrap().station == "connected" {
            Ok(NETWORK_PATH.try_into().unwrap())
        } else {
            Err(zbus::fdo::Error::UnknownProperty("not connected".into()))
        }
    }
}
struct Ap(Shared);
#[zbus::interface(name = "net.connman.iwd.AccessPoint")]
impl Ap {
    fn start_profile(&self, ssid: &str) -> zbus::fdo::Result<()> {
        if ssid != "configured-ap" {
            return Err(zbus::fdo::Error::Failed("profile unavailable".into()));
        }
        let mut state = self.0.lock().unwrap();
        state.ap = true;
        state.mutations += 1;
        Ok(())
    }
    fn start(&self, _ssid: &str, _passphrase: &str) {
        let mut state = self.0.lock().unwrap();
        state.ap = true;
        state.mutations += 1;
    }
    fn stop(&self) {
        let mut state = self.0.lock().unwrap();
        state.ap = false;
        state.mutations += 1;
    }
    #[zbus(property)]
    fn started(&self) -> bool {
        self.0.lock().unwrap().ap
    }
}
struct Manager(Shared);
#[zbus::interface(name = "net.connman.iwd.AgentManager")]
impl Manager {
    fn register_agent(&self, path: OwnedObjectPath, #[zbus(header)] header: Header<'_>) {
        self.0.lock().unwrap().agent = Some((header.sender().unwrap().to_string(), path));
    }
    fn unregister_agent(&self, _path: OwnedObjectPath) {
        self.0.lock().unwrap().agent = None;
    }
}
struct Saved {
    security: &'static str,
    name: &'static str,
    path: &'static str,
}
#[zbus::interface(name = "net.connman.iwd.KnownNetwork")]
impl Saved {
    async fn forget(&self, #[zbus(object_server)] server: &ObjectServer) -> zbus::fdo::Result<()> {
        server
            .remove::<Saved, _>(self.path)
            .await
            .map_err(|e| zbus::fdo::Error::Failed(e.to_string()))?;
        Ok(())
    }
    #[zbus(property)]
    fn name(&self) -> &str {
        self.name
    }
    #[zbus(property, name = "Type")]
    fn security(&self) -> &str {
        self.security
    }
    #[zbus(property)]
    fn auto_connect(&self) -> bool {
        true
    }
}
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "net.connman.iwd")]
enum FakeError {
    Failed(String),
    Aborted(String),
}
struct Visible(Shared);
#[zbus::interface(name = "net.connman.iwd.Network")]
impl Visible {
    async fn connect(&self, #[zbus(connection)] connection: &Connection) -> Result<(), FakeError> {
        let (agent, reject, stall, generation) = {
            let mut state = self.0.lock().unwrap();
            state.connects += 1;
            state.station = "connecting".into();
            state.mutations += 1;
            (
                state.agent.clone(),
                state.reject,
                state.stall,
                state.generation,
            )
        };
        if let Some((destination, path)) = agent {
            let agent = zbus::Proxy::new(connection, destination, path, "net.connman.iwd.Agent")
                .await
                .map_err(|_| FakeError::Failed("agent missing".into()))?;
            let password: String = agent
                .call(
                    "RequestPassphrase",
                    &(OwnedObjectPath::try_from(NETWORK_PATH).unwrap(),),
                )
                .await
                .map_err(|_| FakeError::Failed("agent rejected".into()))?;
            if password != "test-password" {
                return Err(FakeError::Failed("credential rejected".into()));
            }
        }
        if stall {
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        let mut state = self.0.lock().unwrap();
        if state.generation != generation {
            return Err(FakeError::Aborted("cancelled".into()));
        }
        if reject {
            state.station = "disconnected".into();
            return Err(FakeError::Failed(
                "generic failure, not proof of authentication failure".into(),
            ));
        }
        state.station = "connected".into();
        Ok(())
    }
    #[zbus(property)]
    fn name(&self) -> &str {
        "test-network"
    }
    #[zbus(property, name = "Type")]
    fn security(&self) -> &str {
        "psk"
    }
    #[zbus(property)]
    fn device(&self) -> OwnedObjectPath {
        DEVICE_PATH.try_into().unwrap()
    }
}
struct Addresses(Arc<AtomicBool>);
impl AddressSource for Addresses {
    fn has_usable_address(&self, interface: &str) -> Result<bool, Error> {
        assert_eq!(interface, "test-wlan");
        Ok(self.0.load(Ordering::SeqCst))
    }
}

async fn export(server: &Connection, state: &Shared) {
    let objects = server.object_server();
    objects.at("/", zbus::fdo::ObjectManager).await.unwrap();
    objects.at(MANAGER, Manager(state.clone())).await.unwrap();
    objects
        .at(DEVICE_PATH, Device(state.clone()))
        .await
        .unwrap();
    objects
        .at(DEVICE_PATH, Station(state.clone()))
        .await
        .unwrap();
    objects.at(DEVICE_PATH, Ap(state.clone())).await.unwrap();
    objects
        .at(NETWORK_PATH, Visible(state.clone()))
        .await
        .unwrap();
}
fn credentials() -> Credentials {
    Credentials {
        ssid: "test-network".into(),
        password: "test-password".into(),
    }
}
fn target() -> Network {
    credential_network(&credentials())
}
struct Harness {
    _bus: PrivateBus,
    server: Connection,
    state: Shared,
    addresses: Arc<AtomicBool>,
    adapter: IwdAdapter<Addresses>,
}
impl Harness {
    async fn new() -> Self {
        let bus = PrivateBus::start().await;
        let server = bus.connect().await;
        let client = bus.connect().await;
        let state = Shared::default();
        export(&server, &state).await;
        server.request_name(SERVICE).await.unwrap();
        let addresses = Arc::new(AtomicBool::new(true));
        let adapter = IwdAdapter::new(
            client,
            "test-wlan".into(),
            Addresses(addresses.clone()),
            Options {
                operation_timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(5),
                access_point: Some(AccessPointConfig::Credentials(credentials())),
            },
        )
        .unwrap();
        Self {
            _bus: bus,
            server,
            state,
            addresses,
            adapter,
        }
    }
    async fn save(&self) {
        self.server
            .object_server()
            .at(
                KNOWN_PATH,
                Saved {
                    security: "psk",
                    name: "test-network",
                    path: KNOWN_PATH,
                },
            )
            .await
            .unwrap();
        self.server
            .object_server()
            .at(
                OTHER_PATH,
                Saved {
                    security: "psk",
                    name: "other-network",
                    path: OTHER_PATH,
                },
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn ap_uses_iwd_profile_and_does_not_fall_back_when_missing() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.adapter.options.access_point = Some(AccessPointConfig::Profile("missing".into()));
        assert_eq!(
            h.adapter
                .apply(OperationId(1), Action::StartAccessPoint)
                .await,
            Err(Error::Internal)
        );
        assert!(!h.state.lock().unwrap().ap);
        h.adapter.cancel_and_reconcile(OperationId(1)).await?;
        h.adapter.options.access_point = Some(AccessPointConfig::Profile("configured-ap".into()));
        assert!(
            h.adapter
                .apply(OperationId(2), Action::StartAccessPoint)
                .await?
                .access_point_active
        );
        assert!(
            !h.adapter
                .apply(OperationId(3), Action::StopAccessPoint)
                .await?
                .access_point_active
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn observes_autoconnection_without_mutation_and_requires_wifi_address() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.state.lock().unwrap().station = "connected".into();
        h.addresses.store(false, Ordering::SeqCst);
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Associated);
        h.addresses.store(true, Ordering::SeqCst);
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Ready);
        h.save().await;
        assert_eq!(h.adapter.known_networks().await?.len(), 2);
        assert_eq!(h.state.lock().unwrap().mutations, 0);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn scan_agent_connection_and_ap_station_transition() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        assert_eq!(h.adapter.scan().await?, vec![target()]);
        assert!(
            h.adapter
                .apply(OperationId(1), Action::StartAccessPoint)
                .await?
                .access_point_active
        );
        assert_eq!(h.adapter.scan().await, Err(Error::Unsupported));
        assert!(
            !h.adapter
                .apply(OperationId(2), Action::StopAccessPoint)
                .await?
                .access_point_active
        );
        assert_eq!(
            h.adapter
                .apply(OperationId(3), Action::Connect(credentials()))
                .await?
                .link,
            LinkState::Ready
        );
        assert!(h.adapter.agent.0.lock().unwrap().is_none());
        assert_eq!(
            h.adapter
                .apply(OperationId(4), Action::Disconnect)
                .await?
                .link,
            LinkState::Disconnected
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn explicit_replacement_discards_only_selected_profile_even_on_failure() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.save().await;
        h.server
            .object_server()
            .at(
                "/net/connman/iwd/known/test_8021x",
                Saved {
                    name: "test-network",
                    security: "8021x",
                    path: "/net/connman/iwd/known/test_8021x",
                },
            )
            .await
            .unwrap();
        assert_eq!(
            h.adapter
                .apply(OperationId(1), Action::Connect(credentials()))
                .await,
            Err(Error::CredentialConflict)
        );
        assert_eq!(h.adapter.known_networks().await?.len(), 3);
        h.state.lock().unwrap().reject = true;
        assert_eq!(
            h.adapter
                .apply(OperationId(2), Action::ReplaceCredentials(credentials()))
                .await,
            Err(Error::Internal)
        );
        assert!(h.adapter.agent.0.lock().unwrap().is_none());
        let remaining = h.adapter.known_networks().await?;
        assert_eq!(remaining.len(), 2);
        assert!(
            remaining
                .iter()
                .any(|saved| saved.network.ssid == "other-network")
        );
        assert!(
            remaining
                .iter()
                .any(|saved| saved.network.ssid == "test-network"
                    && saved.network.security == Security::Enterprise)
        );
        h.adapter.cancel_and_reconcile(OperationId(2)).await?;
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn saved_connect_failure_is_not_misclassified_as_bad_credentials() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.save().await;
        h.state.lock().unwrap().reject = true;
        assert_eq!(
            h.adapter
                .apply(OperationId(1), Action::ConnectKnown(target()))
                .await,
            Err(Error::Internal)
        );
        assert_eq!(h.adapter.known_networks().await?.len(), 2);
        h.adapter.cancel_and_reconcile(OperationId(1)).await?;
        h.adapter
            .apply(OperationId(2), Action::Forget { network: target() })
            .await?;
        assert_eq!(h.adapter.known_networks().await?.len(), 1);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn timeout_requires_reconciliation_and_rejects_stale_cancellation() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.state.lock().unwrap().stall = true;
        h.adapter.options.operation_timeout = Duration::from_millis(150);
        assert_eq!(
            h.adapter
                .apply(OperationId(1), Action::Connect(credentials()))
                .await,
            Err(Error::Timeout)
        );
        assert!(h.adapter.agent.0.lock().unwrap().is_none());
        assert_eq!(
            h.adapter
                .apply(OperationId(2), Action::StartAccessPoint)
                .await,
            Err(Error::Busy)
        );
        assert_eq!(
            h.adapter.cancel_and_reconcile(OperationId(2)).await,
            Err(Error::Busy)
        );
        h.adapter.options.operation_timeout = Duration::from_secs(2);
        assert_eq!(
            h.adapter.cancel_and_reconcile(OperationId(1)).await?.link,
            LinkState::Disconnected
        );
        tokio::time::sleep(Duration::from_millis(450)).await;
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Disconnected);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn owner_and_device_replacement_do_not_reuse_stale_state() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.state.lock().unwrap().station = "connected".into();
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Ready);
        h.server.release_name(SERVICE).await.unwrap();
        assert_eq!(
            h.adapter.snapshot().await?.backend,
            BackendState::Unavailable
        );
        let new_server = h._bus.connect().await;
        let new_state = Shared::default();
        export(&new_server, &new_state).await;
        new_server.request_name(SERVICE).await.unwrap();
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Disconnected);
        new_server
            .object_server()
            .remove::<Device, _>(DEVICE_PATH)
            .await
            .unwrap();
        assert_eq!(
            h.adapter.snapshot().await?.backend,
            BackendState::Unavailable
        );
        new_server
            .object_server()
            .at("/net/connman/iwd/0/7", Device(new_state.clone()))
            .await
            .unwrap();
        new_server
            .object_server()
            .at("/net/connman/iwd/0/7", Station(new_state))
            .await
            .unwrap();
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Disconnected);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn password_callback_rejects_wrong_bus_sender_or_network() {
    bounded(Duration::from_secs(10), async {
        let h = Harness::new().await;
        let frame = h.adapter.frame().await?;
        h.adapter.register_agent(&frame.identity).await?;
        *h.adapter.agent.0.lock().unwrap() = Some(Secret {
            owner: frame.identity.owner,
            network: NETWORK_PATH.try_into().unwrap(),
            password: "test-password".into(),
        });
        let intruder = h._bus.connect().await;
        let destination = h.adapter.connection.unique_name().unwrap();
        let proxy = zbus::Proxy::new(
            &intruder,
            destination,
            h.adapter.agent_path.as_str(),
            "net.connman.iwd.Agent",
        )
        .await
        .unwrap();
        let result: zbus::Result<String> = proxy
            .call(
                "RequestPassphrase",
                &(OwnedObjectPath::try_from(NETWORK_PATH).unwrap(),),
            )
            .await;
        assert!(result.is_err());
        let trusted = zbus::Proxy::new(
            &h.server,
            destination,
            h.adapter.agent_path.as_str(),
            "net.connman.iwd.Agent",
        )
        .await
        .unwrap();
        let result: zbus::Result<String> = trusted
            .call(
                "RequestPassphrase",
                &(OwnedObjectPath::try_from("/wrong").unwrap(),),
            )
            .await;
        assert!(result.is_err());
        assert!(h.adapter.agent.0.lock().unwrap().is_some());
        h.adapter.agent.clear();
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn unresponsive_backend_reports_unknown_and_recovers_on_next_read() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.adapter.options.operation_timeout = Duration::from_millis(75);
        h.state.lock().unwrap().slow_properties = true;
        assert_eq!(
            h.adapter.snapshot().await?.backend,
            BackendState::Unavailable
        );
        h.state.lock().unwrap().slow_properties = false;
        h.adapter.options.operation_timeout = Duration::from_secs(2);
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Disconnected);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn cancelled_mode_change_cannot_arrive_after_reconciliation() {
    bounded(Duration::from_secs(10), async {
        let mut h = Harness::new().await;
        h.state.lock().unwrap().slow_mode = true;
        h.adapter.options.operation_timeout = Duration::from_millis(150);
        assert_eq!(
            h.adapter
                .apply(OperationId(1), Action::StartAccessPoint)
                .await,
            Err(Error::Timeout)
        );
        assert!(h.adapter.pending_call.lock().await.is_some());
        h.adapter.options.operation_timeout = Duration::from_secs(2);
        h.adapter.cancel_and_reconcile(OperationId(1)).await?;
        tokio::time::sleep(Duration::from_millis(550)).await;
        assert_eq!(h.state.lock().unwrap().mode, "station");
        assert!(h.adapter.pending_call.lock().await.is_none());
        assert_eq!(h.adapter.snapshot().await?.link, LinkState::Disconnected);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn actor_and_production_service_complete_through_the_real_iwd_adapter() {
    use crate::{
        provisioner::{Config, Handle},
        service::Service,
    };
    use futures_util::StreamExt;
    use wifi_setupd_zbus_api::{ProvisioningState, WifiSetupProxy};
    bounded(Duration::from_secs(10), async {
        let h = Harness::new().await;
        let exported = h._bus.connect().await;
        let client = h._bus.connect().await;
        let handle = Handle::spawn(
            h.adapter,
            Config {
                startup_grace: Duration::ZERO,
                poll_interval: Duration::from_millis(5),
                operation_timeout: Duration::from_secs(3),
                cleanup_timeout: Duration::from_secs(1),
                automatic: true,
            },
        )
        .unwrap();
        let service = Service::attach(exported.clone(), handle.clone())
            .await
            .unwrap();
        let proxy = WifiSetupProxy::builder(&client)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        let mut completed = proxy.receive_provisioning_completed().await.unwrap();
        let mut status = handle.subscribe();
        while status.borrow().state != ProvisioningState::AwaitingCredentials {
            status.changed().await.unwrap();
        }
        handle.submit(credentials(), false).await.unwrap();
        completed.next().await.unwrap();
        let ready = proxy.status().await.unwrap();
        assert_eq!(ready.state, ProvisioningState::Idle);
        assert_eq!(ready.link, LinkState::Ready);
        assert_eq!(h.state.lock().unwrap().connects, 1);
        assert_eq!(h.state.lock().unwrap().mode, "station");
        assert!(!h.state.lock().unwrap().ap);
        handle.shutdown().await.unwrap();
        drop(service);
        exported.close().await.unwrap();
        Ok(())
    })
    .await
    .unwrap();
}
