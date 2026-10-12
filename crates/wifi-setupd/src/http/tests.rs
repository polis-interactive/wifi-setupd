use super::*;
use crate::{
    provisioner,
    wifi::{Action, Error, KnownNetwork, Network, OperationId, Security, Snapshot},
};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use std::sync::Mutex as StdMutex;
use tower::ServiceExt;
use wifi_setupd_zbus_api::{BackendState, LinkState};
type Data = (Snapshot, Vec<Network>, Vec<&'static str>);
#[derive(Clone)]
struct Radio(Arc<StdMutex<Data>>);
impl Adapter for Radio {
    async fn snapshot(&mut self) -> Result<Snapshot, Error> {
        Ok(self.0.lock().unwrap().0.clone())
    }
    async fn scan(&mut self) -> Result<Vec<Network>, Error> {
        Ok(vec![test_network()])
    }
    async fn known_networks(&mut self) -> Result<Vec<KnownNetwork>, Error> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .1
            .iter()
            .cloned()
            .map(|network| KnownNetwork {
                network,
                auto_connect: true,
            })
            .collect())
    }
    async fn apply(&mut self, _: OperationId, action: Action) -> Result<Snapshot, Error> {
        let mut d = self.0.lock().unwrap();
        match action {
            Action::StartAccessPoint => d.0.access_point_active = true,
            Action::StopAccessPoint => {
                d.0.access_point_active = false;
                d.2.push("stop-ap");
            }
            Action::Connect(c) | Action::ReplaceCredentials(c) => {
                d.2.push("connect");
                if c.password == "wrong-password" {
                    return Err(Error::Authentication);
                }
                d.0.link = LinkState::Ready;
            }
            Action::Forget { network } => {
                d.2.push("forget");
                d.1.retain(|n| n != &network);
            }
            _ => {}
        }
        Ok(d.0.clone())
    }
    async fn cancel_and_reconcile(&mut self, _: OperationId) -> Result<Snapshot, Error> {
        let mut d = self.0.lock().unwrap();
        d.0.access_point_active = false;
        d.0.link = LinkState::Disconnected;
        Ok(d.0.clone())
    }
}
fn test_network() -> Network {
    Network {
        ssid: "Home".into(),
        security: Security::Personal,
    }
}
async fn setup(ready: bool) -> (Router, Radio, Handle<Radio>, String) {
    let radio = Radio(Arc::new(StdMutex::new((
        Snapshot {
            backend: BackendState::Available,
            link: if ready {
                LinkState::Ready
            } else {
                LinkState::Disconnected
            },
            access_point_active: false,
        },
        vec![test_network()],
        vec![],
    ))));
    let h = Handle::spawn(
        radio.clone(),
        provisioner::Config {
            startup_grace: Duration::ZERO,
            poll_interval: Duration::from_millis(10),
            ..Default::default()
        },
    )
    .unwrap();
    let mut watch = h.subscribe();
    while watch.borrow().backend != BackendState::Available
        || (!ready && watch.borrow().state != ProvisioningState::AwaitingCredentials)
    {
        watch.changed().await.unwrap();
    }
    let app = router(
        h.clone(),
        Config {
            web_root: "/tmp/missing-assets".into(),
            ap_name: "Setup".into(),
            management_key: Some("test-management-key".into()),
        },
        "127.0.0.1:8080".into(),
    )
    .unwrap();
    let s: dto::PortalStatus = json(call(&app, "GET", "/api/status", "", "", "").await).await;
    (app, radio, h, s.csrf)
}
async fn call(
    app: &Router,
    method: &str,
    path: &str,
    body: &str,
    csrf: &str,
    key: &str,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "127.0.0.1:8080")
                .header("content-type", "application/json")
                .header("x-wifi-setup-csrf", csrf)
                .header("authorization", format!("Bearer {key}"))
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn json<T: serde::de::DeserializeOwned>(response: Response) -> T {
    serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap()
}
const INPUT: &str = r#"{"ssid":"Home","password":"valid-password","replace":true}"#;
#[tokio::test]
async fn acceptance_precedes_teardown_and_begin_is_idempotent() {
    let (app, radio, h, csrf) = setup(false).await;
    let response = call(&app, "POST", "/api/connect", INPUT, &csrf, "").await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(
        radio.0.lock().unwrap().2.is_empty(),
        "no teardown before acceptance"
    );
    assert_eq!(
        call(&app, "POST", "/api/connect", INPUT, &csrf, "")
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let id: dto::Accepted = json(response).await;
    let path = format!("/api/connect/{}/begin", id.attempt);
    for _ in 0..2 {
        assert_eq!(
            call(&app, "POST", &path, "null", &csrf, "").await.status(),
            StatusCode::ACCEPTED
        );
    }
    let mut watch = h.subscribe();
    while watch.borrow().link != LinkState::Ready {
        watch.changed().await.unwrap();
    }
    assert_eq!(radio.0.lock().unwrap().2, vec!["stop-ap", "connect"]);
    let s: dto::PortalStatus = json(call(&app, "GET", "/api/status", "", "", "").await).await;
    assert_eq!(s.link, "Ready");
    assert_eq!(s.state, "Idle");
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn rejects_cross_origin_missing_csrf_and_invalid_secret_without_echoing_it() {
    let (app, radio, h, csrf) = setup(false).await;
    let request = Request::builder()
        .method("POST")
        .uri("/api/connect")
        .header("host", "evil.example")
        .header("origin", "http://evil.example")
        .body(Body::from(INPUT))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "POST", "/api/connect", INPUT, "", "")
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for body in [
        r#"{"ssid":"Home","password":"secret","replace":false}"#.to_string(),
        "sensitive".repeat(1000),
    ] {
        let response = call(&app, "POST", "/api/connect", &body, &csrf, "").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error: dto::ApiError = json(response).await;
        assert!(!error.error.contains("sensitive"));
        assert!(!error.error.contains("secret"));
    }
    assert!(radio.0.lock().unwrap().2.is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn saved_deletion_requires_key_and_preserves_other_profiles() {
    let (app, radio, h, csrf) = setup(true).await;
    let other = Network {
        ssid: "Unrelated".into(),
        security: Security::Personal,
    };
    radio.0.lock().unwrap().1.push(other.clone());
    let body = r#"{"ssid":"Home","security":"Personal"}"#;
    assert_eq!(
        call(&app, "POST", "/api/forget", body, &csrf, "")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(radio.0.lock().unwrap().1.len(), 2);
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/forget",
            body,
            &csrf,
            "test-management-key"
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    assert_eq!(radio.0.lock().unwrap().1, vec![other]);
    assert_eq!(h.status().link, LinkState::Ready);
    h.shutdown().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn unstarted_submission_expires_without_teardown() {
    let (app, radio, h, csrf) = setup(false).await;
    let accepted: dto::Accepted =
        json(call(&app, "POST", "/api/connect", INPUT, &csrf, "").await).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/connect/{}/begin", accepted.attempt),
            "null",
            &csrf,
            ""
        )
        .await
        .status(),
        StatusCode::GONE
    );
    assert!(radio.0.lock().unwrap().2.is_empty());
    h.shutdown().await.unwrap();
}
