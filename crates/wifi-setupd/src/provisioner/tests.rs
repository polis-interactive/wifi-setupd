use super::*;
use crate::wifi::{KnownNetwork, Security};
use std::sync::Mutex as StdMutex;

#[derive(Clone)]
struct Mock(Arc<StdMutex<Data>>);
struct Data {
    snapshot: Snapshot,
    calls: Vec<String>,
    stall: bool,
    reject: Option<Error>,
    cleanup_blocked: bool,
    ready_during_scan: bool,
    saved: Vec<Network>,
}
impl Mock {
    fn new(link: LinkState) -> Self {
        Self(Arc::new(StdMutex::new(Data {
            snapshot: Snapshot {
                backend: BackendState::Available,
                link,
                access_point_active: false,
            },
            calls: vec![],
            stall: false,
            reject: None,
            cleanup_blocked: false,
            ready_during_scan: false,
            saved: vec![network("test"), network("unrelated")],
        })))
    }
}
fn network(ssid: &str) -> Network {
    Network {
        ssid: ssid.into(),
        security: Security::Personal,
    }
}
fn credentials() -> Credentials {
    Credentials {
        ssid: "test".into(),
        password: "test-password".into(),
    }
}
impl Adapter for Mock {
    async fn snapshot(&mut self) -> Result<Snapshot, Error> {
        Ok(self.0.lock().unwrap().snapshot.clone())
    }
    async fn scan(&mut self) -> Result<Vec<Network>, Error> {
        let mut data = self.0.lock().unwrap();
        data.calls.push("scan".into());
        if data.ready_during_scan {
            data.snapshot.link = LinkState::Ready;
        }
        Ok(vec![network("test")])
    }
    async fn known_networks(&mut self) -> Result<Vec<KnownNetwork>, Error> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .saved
            .iter()
            .map(|network| KnownNetwork {
                network: network.clone(),
                auto_connect: true,
            })
            .collect())
    }
    async fn apply(&mut self, _: OperationId, action: Action) -> Result<Snapshot, Error> {
        let is_connect = matches!(
            action,
            Action::Connect(_) | Action::ReplaceCredentials(_) | Action::ConnectKnown(_)
        );
        {
            let mut data = self.0.lock().unwrap();
            if data.snapshot.backend == BackendState::Unavailable {
                return Err(Error::Unavailable);
            }
            match action {
                Action::StartAccessPoint => {
                    data.calls.push("start-ap".into());
                    data.snapshot.access_point_active = true;
                    data.snapshot.link = LinkState::Disconnected;
                }
                Action::StopAccessPoint => {
                    data.calls.push("stop-ap".into());
                    data.snapshot.access_point_active = false;
                    data.snapshot.link = LinkState::Disconnected;
                }
                Action::Connect(_) | Action::ReplaceCredentials(_) | Action::ConnectKnown(_) => {
                    data.calls.push("connect".into());
                    data.snapshot.link = LinkState::Associating;
                }
                Action::Disconnect => {
                    data.calls.push("disconnect".into());
                    data.snapshot.link = LinkState::Disconnected;
                }
                Action::Forget { network } => {
                    data.calls.push("forget".into());
                    data.saved.retain(|saved| *saved != network);
                }
            }
        }
        let stall = self.0.lock().unwrap().stall;
        if is_connect && stall {
            std::future::pending::<()>().await;
        }
        let mut data = self.0.lock().unwrap();
        if is_connect {
            if let Some(error) = data.reject {
                return Err(error);
            }
            data.snapshot.link = LinkState::Ready;
        }
        Ok(data.snapshot.clone())
    }
    async fn cancel_and_reconcile(&mut self, _: OperationId) -> Result<Snapshot, Error> {
        let mut data = self.0.lock().unwrap();
        data.calls.push("cleanup".into());
        if data.cleanup_blocked || data.snapshot.backend == BackendState::Unavailable {
            return Err(Error::Unavailable);
        }
        data.snapshot.link = LinkState::Disconnected;
        data.snapshot.access_point_active = false;
        Ok(data.snapshot.clone())
    }
}
fn config() -> Config {
    Config {
        startup_grace: Duration::from_millis(500),
        poll_interval: Duration::from_millis(50),
        operation_timeout: Duration::from_secs(1),
        cleanup_timeout: Duration::from_millis(100),
        automatic: true,
    }
}
async fn until(handle: &Handle<Mock>, predicate: impl Fn(Status) -> bool) -> Status {
    let mut status = handle.subscribe();
    timeout(Duration::from_secs(5), async {
        loop {
            let value = *status.borrow_and_update();
            if predicate(value) {
                return value;
            }
            status.changed().await.unwrap();
        }
    })
    .await
    .expect("expected actor state within 5 seconds")
}
async fn pump() {
    for _ in 0..30 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn auto_connection_finishing_during_scan_prevents_ap_start() {
    let mock = Mock::new(LinkState::Associating);
    mock.0.lock().unwrap().ready_during_scan = true;
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.link == LinkState::Ready).await;
    assert_eq!(handle.status().state, State::Idle);
    assert_eq!(mock.0.lock().unwrap().calls, ["scan"]);
    handle.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn first_boot_grace_idempotent_start_and_persistent_ap_until_explicit_stop() {
    let mock = Mock::new(LinkState::Disconnected);
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.backend == BackendState::Available).await;
    assert_eq!(handle.status().state, State::Idle);
    assert!(mock.0.lock().unwrap().calls.is_empty());
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    handle.start().await.unwrap();
    handle.start().await.unwrap();
    tokio::time::advance(Duration::from_secs(600)).await;
    pump().await;
    assert_eq!(handle.status().state, State::AwaitingCredentials);
    assert_eq!(
        mock.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| *c == "start-ap")
            .count(),
        1
    );
    handle.stop().await.unwrap();
    handle.stop().await.unwrap();
    assert_eq!(
        until(&handle, |s| s.state == State::Idle).await.last_error,
        ProvisioningError::Cancelled
    );
    tokio::time::advance(Duration::from_secs(30)).await;
    pump().await;
    assert_eq!(handle.status().state, State::Idle);
    handle.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn observes_saved_autoconnection_and_suppresses_ap_only_for_wifi_ready() {
    let mock = Mock::new(LinkState::Associating);
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.link == LinkState::Associating).await;
    mock.0.lock().unwrap().snapshot.link = LinkState::Ready;
    until(&handle, |s| s.link == LinkState::Ready).await;
    handle.start().await.unwrap();
    tokio::time::advance(Duration::from_secs(40)).await;
    pump().await;
    assert_eq!(handle.status().state, State::Idle);
    assert!(mock.0.lock().unwrap().calls.is_empty());
    mock.0.lock().unwrap().snapshot.link = LinkState::Associated;
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    handle.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn successful_attempt_publishes_ready_then_exactly_one_completion_and_stays_callable() {
    let mock = Mock::new(LinkState::Disconnected);
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    let mut events = handle.publications();
    handle.submit(credentials(), false).await.unwrap();
    until(&handle, |s| {
        s.state == State::Idle && s.link == LinkState::Ready
    })
    .await;
    handle.start().await.unwrap();
    handle.stop().await.unwrap();
    let mut completions = 0;
    let mut verifying = false;
    while let Ok(event) = events.try_recv() {
        verifying |= event.status.state == State::Verifying;
        if event.completed {
            completions += 1;
            assert_eq!(event.status.link, LinkState::Ready);
            assert_eq!(event.status.state, State::Idle);
        }
    }
    assert!(verifying);
    assert_eq!(completions, 1);
    let calls = mock.0.lock().unwrap().calls.clone();
    assert!(
        calls
            .windows(3)
            .any(|s| s == ["stop-ap", "scan", "connect"])
    );
    assert!(!calls.iter().any(|call| call == "disconnect"));
    handle.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_is_responsive_and_late_results_cannot_complete_cancelled_work() {
    let mock = Mock::new(LinkState::Disconnected);
    mock.0.lock().unwrap().stall = true;
    mock.0.lock().unwrap().cleanup_blocked = true;
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    let mut events = handle.publications();
    handle.submit(credentials(), false).await.unwrap();
    pump().await;
    handle.stop().await.unwrap();
    pump().await;
    assert_eq!(handle.status().state, State::Stopping);
    assert_eq!(handle.start().await, Err(CommandError::Busy));
    handle.stop().await.unwrap();
    // A result with an obsolete operation id must not change current state.
    handle
        .actor
        .tell(Finished {
            id: OperationId(0),
            result: Ok(Snapshot {
                backend: BackendState::Available,
                link: LinkState::Ready,
                access_point_active: false,
            }),
            cleanup: Err(Error::Internal),
        })
        .await
        .unwrap();
    pump().await;
    assert_eq!(handle.status().state, State::Stopping);
    mock.0.lock().unwrap().cleanup_blocked = false;
    let status = until(&handle, |s| s.state == State::Idle).await;
    assert_eq!(status.last_error, ProvisioningError::Cancelled);
    while let Ok(event) = events.try_recv() {
        assert!(!event.completed);
    }
    handle.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn timeout_reconciles_and_restores_portal_without_erasing_attempt_error() {
    let mock = Mock::new(LinkState::Disconnected);
    mock.0.lock().unwrap().stall = true;
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    handle.submit(credentials(), false).await.unwrap();
    until(&handle, |s| {
        s.state == State::AwaitingCredentials && s.last_error == ProvisioningError::TimedOut
    })
    .await;
    handle.start().await.unwrap();
    assert_eq!(handle.status().last_error, ProvisioningError::TimedOut);
    assert!(
        mock.0
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| call == "cleanup")
    );
    mock.0.lock().unwrap().stall = false;
    handle.submit(credentials(), true).await.unwrap();
    until(&handle, |s| s.link == LinkState::Ready).await;
    assert_eq!(handle.status().last_error, ProvisioningError::None);
    handle.shutdown().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn confirmed_saved_authentication_failure_forgets_only_selected_network() {
    for error in [Error::Authentication, Error::Internal] {
        let mock = Mock::new(LinkState::Disconnected);
        mock.0.lock().unwrap().reject = Some(error);
        let handle = Handle::spawn(mock.clone(), config()).unwrap();
        until(&handle, |s| s.state == State::AwaitingCredentials).await;
        handle.connect_known(network("test")).await.unwrap();
        until(&handle, |s| {
            s.state == State::AwaitingCredentials && s.last_error != ProvisioningError::None
        })
        .await;
        let saved = mock.0.lock().unwrap().saved.clone();
        assert!(saved.contains(&network("unrelated")));
        assert_eq!(
            saved.contains(&network("test")),
            error != Error::Authentication
        );
        handle.shutdown().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn backend_loss_removes_readiness_and_recovers_from_fresh_observations() {
    let mock = Mock::new(LinkState::Ready);
    let handle = Handle::spawn(mock.clone(), config()).unwrap();
    until(&handle, |s| s.link == LinkState::Ready).await;
    mock.0.lock().unwrap().snapshot.backend = BackendState::Unavailable;
    until(&handle, |s| s.backend == BackendState::Unavailable).await;
    assert_eq!(handle.status().link, LinkState::Unknown);
    assert_eq!(handle.start().await, Err(CommandError::BackendUnavailable));
    {
        let mut data = mock.0.lock().unwrap();
        data.snapshot.backend = BackendState::Available;
        data.snapshot.link = LinkState::Disconnected;
    }
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    mock.0.lock().unwrap().snapshot.backend = BackendState::Unavailable;
    until(&handle, |s| s.state == State::Stopping).await;
    mock.0.lock().unwrap().snapshot.backend = BackendState::Available;
    until(&handle, |s| s.state == State::AwaitingCredentials).await;
    handle.shutdown().await.unwrap();
}

use crate::test_bus;

#[tokio::test]
async fn production_dbus_exports_properties_completion_and_fresh_status_after_reconnect() {
    use crate::service::Service;
    use futures_util::StreamExt;
    use wifi_setupd_zbus_api::{OBJECT_PATH, WifiSetupProxy};
    timeout(Duration::from_secs(10), async {
        let bus = test_bus::PrivateBus::start().await;
        let server = bus.connect().await;
        let mock = Mock::new(LinkState::Disconnected);
        let handle = Handle::spawn(
            mock,
            Config {
                automatic: false,
                ..config()
            },
        )
        .unwrap();
        until(&handle, |s| s.backend == BackendState::Available).await;
        let service = Service::attach(server.clone(), handle.clone())
            .await
            .unwrap();
        let client = bus.connect().await;
        let proxy = WifiSetupProxy::builder(&client)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        let introspection = zbus::fdo::IntrospectableProxy::builder(&client)
            .destination(wifi_setupd_zbus_api::BUS_NAME)
            .unwrap()
            .path(OBJECT_PATH)
            .unwrap()
            .build()
            .await
            .unwrap()
            .introspect()
            .await
            .unwrap();
        assert!(introspection.contains("name=\"Status\" type=\"(ssss)\" access=\"read\""));
        let mut completed = proxy.receive_provisioning_completed().await.unwrap();
        let cached = WifiSetupProxy::new(&client).await.unwrap();
        let mut changed = cached.receive_status_changed().await;
        proxy.start_provisioning().await.unwrap();
        proxy.start_provisioning().await.unwrap();
        until(&handle, |s| s.state == State::AwaitingCredentials).await;
        changed.next().await.unwrap().get().await.unwrap();
        handle.submit(credentials(), false).await.unwrap();
        completed.next().await.unwrap();
        let status = proxy.status().await.unwrap();
        assert_eq!(status.state, State::Idle);
        assert_eq!(status.link, LinkState::Ready);
        assert_eq!(status.last_error, ProvisioningError::None);
        assert_eq!(proxy.link_state().await.unwrap(), LinkState::Ready);
        assert!(!proxy.provisioning_active().await.unwrap());
        proxy.start_provisioning().await.unwrap();
        proxy.stop_provisioning().await.unwrap();
        assert!(
            timeout(Duration::from_millis(30), completed.next())
                .await
                .is_err()
        );
        drop(service);
        server.close().await.unwrap();
        // Re-export the same running actor on a new connection; completion is
        // not replayed, while properties reconstruct the current outcome.
        let server = bus.connect().await;
        let service = Service::attach(server.clone(), handle.clone())
            .await
            .unwrap();
        let new_client = bus.connect().await;
        let reconnected = WifiSetupProxy::new(&new_client).await.unwrap();
        assert_eq!(reconnected.status().await.unwrap(), status);
        let mut new_completed = reconnected.receive_provisioning_completed().await.unwrap();
        assert!(
            timeout(Duration::from_millis(30), new_completed.next())
                .await
                .is_err()
        );
        handle.shutdown().await.unwrap();
        drop(service);
        server.close().await.unwrap();
    })
    .await
    .expect("production D-Bus acceptance exceeded 10 seconds");
}

#[tokio::test]
async fn production_dbus_stop_does_not_wait_for_the_connection_worker() {
    use wifi_setupd_zbus_api::{ControlError, WifiSetupProxy};
    timeout(Duration::from_secs(10), async {
        let bus = test_bus::PrivateBus::start().await;
        let server = bus.connect().await;
        let mock = Mock::new(LinkState::Disconnected);
        mock.0.lock().unwrap().stall = true;
        mock.0.lock().unwrap().cleanup_blocked = true;
        let handle = Handle::spawn(
            mock.clone(),
            Config {
                startup_grace: Duration::ZERO,
                operation_timeout: Duration::from_secs(30),
                ..config()
            },
        )
        .unwrap();
        until(&handle, |s| s.state == State::AwaitingCredentials).await;
        let service = crate::service::Service::attach(server.clone(), handle.clone())
            .await
            .unwrap();
        let client = bus.connect().await;
        let proxy = WifiSetupProxy::builder(&client)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await
            .unwrap();
        handle.submit(credentials(), false).await.unwrap();
        pump().await;
        timeout(Duration::from_millis(500), proxy.stop_provisioning())
            .await
            .expect("Stop blocked on adapter")
            .unwrap();
        assert!(matches!(
            proxy.start_provisioning().await,
            Err(ControlError::Busy(_))
        ));
        mock.0.lock().unwrap().cleanup_blocked = false;
        until(&handle, |s| s.state == State::Idle).await;
        handle.shutdown().await.unwrap();
        drop(service);
        server.close().await.unwrap();
    })
    .await
    .expect("D-Bus cancellation test exceeded 10 seconds");
}
