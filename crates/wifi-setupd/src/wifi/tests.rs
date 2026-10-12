use super::*;
use std::{collections::VecDeque, path::PathBuf, time::Duration};
use wifi_setupd_api::{ProvisioningState, Status};

#[derive(Debug, PartialEq, Eq)]
enum Request {
    Snapshot,
    Scan,
    Apply(OperationId, Action),
    Cancel(OperationId),
}

enum Reply {
    Snapshot(Result<Snapshot, Error>),
    Networks(Vec<Network>),
}

struct Step {
    request: Request,
    reply: Reply,
    delay: Duration,
}

/// Strict script: an unexpected call fails rather than touching a host service.
/// The directory is injected and contains only mock metadata, never passwords.
struct ScriptedAdapter {
    steps: VecDeque<Step>,
    persistence: PathBuf,
}

impl ScriptedAdapter {
    async fn next(&mut self, request: Request) -> Reply {
        let step = self.steps.pop_front().expect("unexpected adapter call");
        assert_eq!(step.request, request);
        tokio::time::sleep(step.delay).await;
        if let (Request::Apply(_, Action::Connect(_)), Reply::Snapshot(Ok(snapshot))) =
            (&request, &step.reply)
            && snapshot.link == LinkState::Ready
        {
            std::fs::write(self.persistence.join("mock-connected"), b"ready").unwrap();
        }
        step.reply
    }

    async fn state_reply(&mut self, request: Request) -> Result<Snapshot, Error> {
        match self.next(request).await {
            Reply::Snapshot(result) => result,
            _ => panic!("expected a snapshot reply"),
        }
    }

    fn assert_finished(&self) {
        assert!(self.steps.is_empty(), "unconsumed adapter script");
    }
}

impl Adapter for ScriptedAdapter {
    async fn known_networks(&mut self) -> Result<Vec<KnownNetwork>, Error> {
        panic!("known-network lookup is not in this script")
    }
    async fn snapshot(&mut self) -> Result<Snapshot, Error> {
        self.state_reply(Request::Snapshot).await
    }
    async fn scan(&mut self) -> Result<Vec<Network>, Error> {
        match self.next(Request::Scan).await {
            Reply::Networks(networks) => Ok(networks),
            _ => panic!("expected scan reply"),
        }
    }
    async fn apply(&mut self, operation: OperationId, action: Action) -> Result<Snapshot, Error> {
        self.state_reply(Request::Apply(operation, action)).await
    }
    async fn cancel_and_reconcile(&mut self, operation: OperationId) -> Result<Snapshot, Error> {
        self.state_reply(Request::Cancel(operation)).await
    }
}

fn snapshot(link: LinkState, ap: bool) -> Snapshot {
    Snapshot {
        backend: BackendState::Available,
        link,
        access_point_active: ap,
    }
}

fn step(request: Request, result: Result<Snapshot, Error>) -> Step {
    Step {
        request,
        reply: Reply::Snapshot(result),
        delay: Duration::ZERO,
    }
}

fn connect() -> Action {
    Action::Connect(Credentials {
        ssid: "test-network".into(),
        password: "test-secret".into(),
    })
}

#[tokio::test(start_paused = true)]
async fn simulated_wifi_publishes_readiness_with_temporary_storage() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_path_buf();
    let operation = OperationId(1);
    let mut adapter = ScriptedAdapter {
        persistence: path.clone(),
        steps: VecDeque::from([
            step(
                Request::Snapshot,
                Ok(snapshot(LinkState::Disconnected, false)),
            ),
            Step {
                request: Request::Scan,
                reply: Reply::Networks(vec![Network {
                    ssid: "test-network".into(),
                    security: Security::Personal,
                }]),
                delay: Duration::ZERO,
            },
            step(
                Request::Apply(operation, Action::StartAccessPoint),
                Ok(snapshot(LinkState::Disconnected, true)),
            ),
            step(
                Request::Apply(operation, Action::StopAccessPoint),
                Ok(snapshot(LinkState::Disconnected, false)),
            ),
            step(
                Request::Apply(operation, connect()),
                Ok(snapshot(LinkState::Ready, false)),
            ),
        ]),
    };
    let (sender, mut receiver) = crate::status::channel(Status::default());
    assert_eq!(receiver.borrow().link, LinkState::Unknown);
    assert!(!receiver.borrow().provisioning_active());
    assert_eq!(
        adapter.snapshot().await.unwrap().link,
        LinkState::Disconnected
    );
    assert_eq!(adapter.scan().await.unwrap()[0].ssid, "test-network");
    assert!(
        adapter
            .apply(operation, Action::StartAccessPoint)
            .await
            .unwrap()
            .access_point_active
    );
    sender.send_modify(|s| {
        s.state = ProvisioningState::AwaitingCredentials;
        s.backend = BackendState::Available;
        s.link = LinkState::Disconnected;
    });
    receiver.changed().await.unwrap();
    assert!(receiver.borrow_and_update().provisioning_active());
    adapter
        .apply(operation, Action::StopAccessPoint)
        .await
        .unwrap();
    let observed = adapter.apply(operation, connect()).await.unwrap();
    sender.send_replace(Status {
        state: ProvisioningState::Idle,
        link: observed.link,
        backend: observed.backend,
        last_error: ProvisioningError::None,
    });
    receiver.changed().await.unwrap();
    assert_eq!(receiver.borrow().link, LinkState::Ready);
    assert!(!receiver.borrow().provisioning_active());
    assert_eq!(
        std::fs::read(path.join("mock-connected")).unwrap(),
        b"ready"
    );
    adapter.assert_finished();
    drop(directory);
    assert!(!path.exists());
}

#[tokio::test(start_paused = true)]
async fn failure_and_timeout_preserve_storage_and_require_reconciliation() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("mock-connected");
    std::fs::write(&marker, b"previous").unwrap();
    let operation = OperationId(2);
    let mut delayed = step(
        Request::Apply(operation, connect()),
        Ok(snapshot(LinkState::Ready, false)),
    );
    delayed.delay = Duration::from_secs(60);
    let mut adapter = ScriptedAdapter {
        persistence: directory.path().into(),
        steps: VecDeque::from([
            step(
                Request::Apply(OperationId(1), connect()),
                Err(Error::Authentication),
            ),
            delayed,
            step(
                Request::Cancel(operation),
                Ok(snapshot(LinkState::Disconnected, false)),
            ),
        ]),
    };
    let error = adapter.apply(OperationId(1), connect()).await.unwrap_err();
    assert_eq!(
        ProvisioningError::from(error),
        ProvisioningError::AuthenticationFailed
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(5), adapter.apply(operation, connect()))
            .await
            .is_err()
    );
    assert_eq!(
        adapter.cancel_and_reconcile(operation).await.unwrap().link,
        LinkState::Disconnected
    );
    assert_eq!(std::fs::read(marker).unwrap(), b"previous");
    adapter.assert_finished();
}

#[test]
fn credential_debug_is_redacted() {
    let debug = format!("{:?}", connect());
    assert!(!debug.contains("test-secret"));
    assert!(!debug.contains("test-network"));
}
