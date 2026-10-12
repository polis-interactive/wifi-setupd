mod support;

use futures_util::StreamExt;
use wifi_setupd_api::{
    BUS_NAME, BackendState, ControlError, INTERFACE_NAME, LinkState, OBJECT_PATH,
    ProvisioningError, ProvisioningState, Status, WifiSetupProxy,
};
use zbus::{object_server::SignalEmitter, zvariant::Type};

/// Contract fixture only: no actor or real network workflow is implemented here.
#[derive(Default)]
struct Fixture {
    status: Status,
}

impl Fixture {
    async fn publish(&self, emitter: &SignalEmitter<'_>) -> zbus::Result<()> {
        self.status_changed(emitter).await?;
        self.state_changed(emitter).await?;
        self.provisioning_active_changed(emitter).await?;
        self.link_state_changed(emitter).await?;
        self.backend_state_changed(emitter).await?;
        self.last_error_changed(emitter).await
    }
}

#[zbus::interface(name = "tv.polis.WifiSetup1")]
impl Fixture {
    async fn start_provisioning(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), ControlError> {
        if self.status.state == ProvisioningState::Stopping {
            return Err(ControlError::Busy(
                "Cancellation is still in progress".into(),
            ));
        }
        if self.status.provisioning_active() {
            return Ok(());
        }
        if self.status.backend == BackendState::Unavailable {
            return Err(ControlError::BackendUnavailable(
                "Wi-Fi backend unavailable".into(),
            ));
        }
        if self.status.link == LinkState::Ready {
            return Ok(());
        }
        self.status.state = ProvisioningState::StartingAccessPoint;
        self.status.last_error = ProvisioningError::None;
        self.publish(&emitter).await?;
        Ok(())
    }

    async fn stop_provisioning(
        &mut self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> Result<(), ControlError> {
        if self.status.provisioning_active() && self.status.state != ProvisioningState::Stopping {
            self.status.state = ProvisioningState::Stopping;
            self.publish(&emitter).await?;
        }
        Ok(())
    }

    #[zbus(property)]
    fn state(&self) -> ProvisioningState {
        self.status.state
    }
    #[zbus(property)]
    fn provisioning_active(&self) -> bool {
        self.status.provisioning_active()
    }
    #[zbus(property)]
    fn last_error(&self) -> ProvisioningError {
        self.status.last_error
    }
    #[zbus(property)]
    fn link_state(&self) -> LinkState {
        self.status.link
    }
    #[zbus(property)]
    fn backend_state(&self) -> BackendState {
        self.status.backend
    }
    #[zbus(property)]
    fn status(&self) -> Status {
        self.status
    }
    #[zbus(signal)]
    async fn provisioning_completed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[tokio::test]
async fn private_bus_contract() {
    tokio::time::timeout(std::time::Duration::from_secs(10), contract())
        .await
        .expect("contract test exceeded 10 seconds");
}

async fn contract() {
    let bus = support::PrivateBus::start().await;
    let server = bus.connect().await;
    server
        .object_server()
        .at(OBJECT_PATH, Fixture::default())
        .await
        .unwrap();
    server.request_name(BUS_NAME).await.unwrap();
    let client = bus.connect().await;
    let proxy = WifiSetupProxy::new(&client).await.unwrap();
    assert_eq!(proxy.status().await.unwrap(), Status::default());
    assert_eq!(proxy.state().await.unwrap(), ProvisioningState::Idle);
    assert_eq!(proxy.link_state().await.unwrap(), LinkState::Unknown);
    assert_eq!(
        proxy.backend_state().await.unwrap(),
        BackendState::Unavailable
    );
    assert_eq!(proxy.last_error().await.unwrap(), ProvisioningError::None);
    assert!(!proxy.provisioning_active().await.unwrap());
    assert!(matches!(
        proxy.start_provisioning().await,
        Err(ControlError::BackendUnavailable(_))
    ));
    proxy.stop_provisioning().await.unwrap();
    assert_eq!(proxy.status().await.unwrap(), Status::default());

    // Check names and wire signatures independently of the typed client.
    let introspect = zbus::fdo::IntrospectableProxy::builder(&client)
        .destination(BUS_NAME)
        .unwrap()
        .path(OBJECT_PATH)
        .unwrap()
        .build()
        .await
        .unwrap();
    let xml = introspect.introspect().await.unwrap();
    assert!(xml.contains(INTERFACE_NAME));
    for property in ["State", "LastError", "LinkState", "BackendState"] {
        assert!(
            xml.contains(&format!("name=\"{property}\" type=\"s\" access=\"read\"")),
            "{xml}"
        );
    }
    assert!(xml.contains("name=\"Status\" type=\"(ssss)\" access=\"read\""));
    assert!(xml.contains("name=\"ProvisioningActive\" type=\"b\" access=\"read\""));
    assert!(xml.contains("method name=\"StartProvisioning\""));
    assert!(xml.contains("method name=\"StopProvisioning\""));
    assert!(xml.contains("signal name=\"ProvisioningCompleted\""));

    let interface = server
        .object_server()
        .interface::<_, Fixture>(OBJECT_PATH)
        .await
        .unwrap();
    {
        let mut fixture = interface.get_mut().await;
        fixture.status.backend = BackendState::Available;
        fixture.status.link = LinkState::Disconnected;
        fixture.status.last_error = ProvisioningError::AuthenticationFailed;
        fixture.publish(interface.signal_emitter()).await.unwrap();
    }
    // Disable property caching when asserting immediate command postconditions.
    let fresh = WifiSetupProxy::builder(&client)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .unwrap();
    let mut updates = proxy.receive_state_changed().await;
    proxy.start_provisioning().await.unwrap();
    loop {
        let update = updates.next().await.unwrap();
        if update.get().await.unwrap() == ProvisioningState::StartingAccessPoint {
            break;
        }
    }
    assert_eq!(
        fresh.state().await.unwrap(),
        ProvisioningState::StartingAccessPoint
    );
    assert_eq!(fresh.last_error().await.unwrap(), ProvisioningError::None);
    assert!(fresh.provisioning_active().await.unwrap());
    proxy.start_provisioning().await.unwrap();
    assert_eq!(
        fresh.state().await.unwrap(),
        ProvisioningState::StartingAccessPoint
    );
    proxy.stop_provisioning().await.unwrap();
    proxy.stop_provisioning().await.unwrap();
    assert_eq!(fresh.state().await.unwrap(), ProvisioningState::Stopping);
    assert!(matches!(
        proxy.start_provisioning().await,
        Err(ControlError::Busy(_))
    ));
    {
        let mut fixture = interface.get_mut().await;
        fixture.status.state = ProvisioningState::Idle;
        fixture.status.last_error = ProvisioningError::Cancelled;
        fixture.publish(interface.signal_emitter()).await.unwrap();
    }
    proxy.stop_provisioning().await.unwrap();
    assert_eq!(
        fresh.last_error().await.unwrap(),
        ProvisioningError::Cancelled
    );
    proxy.start_provisioning().await.unwrap();
    assert_eq!(fresh.last_error().await.unwrap(), ProvisioningError::None);

    let mut completed = proxy.receive_provisioning_completed().await.unwrap();
    {
        let mut fixture = interface.get_mut().await;
        fixture.status = Status {
            state: ProvisioningState::Idle,
            link: LinkState::Ready,
            backend: BackendState::Available,
            last_error: ProvisioningError::None,
        };
        fixture.publish(interface.signal_emitter()).await.unwrap();
        Fixture::provisioning_completed(interface.signal_emitter())
            .await
            .unwrap();
    }
    completed.next().await.unwrap();
    assert_eq!(fresh.status().await.unwrap().link, LinkState::Ready);
    assert!(!fresh.provisioning_active().await.unwrap());
    // A newly connected client reconstructs status without a replayed signal.
    let reconnected = bus.connect().await;
    let new_proxy = WifiSetupProxy::new(&reconnected).await.unwrap();
    assert_eq!(
        new_proxy.status().await.unwrap(),
        fresh.status().await.unwrap()
    );
}

#[test]
fn wire_values_are_strings_and_snapshot_order_is_stable() {
    use zbus::zvariant::{LE, serialized::Context, to_bytes};
    assert_eq!(Status::SIGNATURE.to_string(), "(ssss)");
    let status = Status {
        state: ProvisioningState::Stopping,
        link: LinkState::Associated,
        backend: BackendState::Available,
        last_error: ProvisioningError::AuthenticationFailed,
    };
    let bytes = to_bytes(Context::new_dbus(LE, 0), &status).unwrap();
    let (wire, _): ((String, String, String, String), _) = bytes.deserialize().unwrap();
    assert_eq!(
        wire,
        (
            "Stopping".into(),
            "Associated".into(),
            "Available".into(),
            "AuthenticationFailed".into()
        )
    );
    assert_eq!(bytes.deserialize::<Status>().unwrap().0, status);
    let unknown = to_bytes(Context::new_dbus(LE, 0), &"FutureState").unwrap();
    assert!(unknown.deserialize::<ProvisioningState>().is_err());
}
