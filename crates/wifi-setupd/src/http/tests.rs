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
                d.0.connected_ssid = Some(c.ssid);
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
            connected_ssid: ready.then(|| "Home".into()),
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
            device_key: Some("test-device-password".into()),
            hostnames: vec![],
        },
        "127.0.0.1:8080".into(),
    )
    .unwrap();
    let s: dto::PortalStatus = json(call(&app, "GET", "/api/status", "", "").await).await;
    (app, radio, h, s.csrf)
}
async fn call(app: &Router, method: &str, path: &str, body: &str, csrf: &str) -> Response {
    call_with_key(app, method, path, body, csrf, "").await
}
async fn call_with_key(
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
    let response = call(&app, "POST", "/api/connect", INPUT, &csrf).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(
        radio.0.lock().unwrap().2.is_empty(),
        "no teardown before acceptance"
    );
    assert_eq!(
        call(&app, "POST", "/api/connect", INPUT, &csrf)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let id: dto::Accepted = json(response).await;
    let path = format!("/api/connect/{}/begin", id.attempt);
    for _ in 0..2 {
        assert_eq!(
            call(&app, "POST", &path, "null", &csrf).await.status(),
            StatusCode::ACCEPTED
        );
    }
    let mut watch = h.subscribe();
    while watch.borrow().link != LinkState::Ready {
        watch.changed().await.unwrap();
    }
    assert_eq!(radio.0.lock().unwrap().2, vec!["stop-ap", "connect"]);
    let s: dto::PortalStatus = json(call(&app, "GET", "/api/status", "", "").await).await;
    assert_eq!(s.link, "Ready");
    assert_eq!(s.connected_ssid.as_deref(), Some("Home"));
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
        call(&app, "POST", "/api/connect", INPUT, "").await.status(),
        StatusCode::FORBIDDEN
    );
    for body in [
        r#"{"ssid":"Home","password":"secret","replace":false}"#.to_string(),
        "sensitive".repeat(1000),
    ] {
        let response = call(&app, "POST", "/api/connect", &body, &csrf).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error: dto::ApiError = json(response).await;
        assert!(!error.error.contains("sensitive"));
        assert!(!error.error.contains("secret"));
    }
    assert!(radio.0.lock().unwrap().2.is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn saved_deletion_requires_device_password_and_preserves_other_profiles() {
    let (app, radio, h, csrf) = setup(true).await;
    let other = Network {
        ssid: "Unrelated".into(),
        security: Security::Personal,
    };
    radio.0.lock().unwrap().1.push(other.clone());
    let body = r#"{"ssid":"Home","security":"Personal"}"#;
    assert_eq!(
        call(&app, "POST", "/api/forget", body, "").await.status(),
        StatusCode::FORBIDDEN
    );
    for key in ["", "wrong-device-password"] {
        assert_eq!(
            call_with_key(&app, "POST", "/api/forget", body, &csrf, key)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(radio.0.lock().unwrap().1.len(), 2);
    assert_eq!(
        call_with_key(
            &app,
            "POST",
            "/api/forget",
            body,
            &csrf,
            "test-device-password"
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
        json(call(&app, "POST", "/api/connect", INPUT, &csrf).await).await;
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
        )
        .await
        .status(),
        StatusCode::GONE
    );
    assert!(radio.0.lock().unwrap().2.is_empty());
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn captive_probes_redirect_only_during_ap_onboarding_without_exposing_api() {
    let (app, _, h, _) = setup(false).await;
    for (host, path) in [
        ("captive.apple.com", "/hotspot-detect.html"),
        ("www.apple.com", "/library/test/success.html"),
        ("connectivitycheck.gstatic.com", "/generate_204"),
        ("www.msftconnecttest.com", "/connecttest.txt"),
        ("example.com", "/arbitrary?token=do-not-reflect"),
        ("127.0.0.1:8080", "/hotspot-detect.html"),
    ] {
        let request = Request::builder()
            .uri(path)
            .header("host", host)
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()["location"], "http://127.0.0.1:8080/");
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    for (method, path) in [
        ("POST", "/api/connect"),
        ("GET", "/api/status"),
        ("POST", "/hotspot-detect.html"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "captive.apple.com")
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    let request = Request::builder()
        .uri("/hotspot-detect.html")
        .header("host", "captive.apple.com")
        .header("origin", "http://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    h.stop().await.unwrap();
    let mut status = h.subscribe();
    while status.borrow().state.is_active() {
        status.changed().await.unwrap();
    }
    let request = Request::builder()
        .uri("/hotspot-detect.html")
        .header("host", "captive.apple.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    h.shutdown().await.unwrap();
    let (app, _, h, _) = setup(true).await;
    let request = Request::builder()
        .uri("/hotspot-detect.html")
        .header("host", "captive.apple.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(request).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn configured_hostname_serves_portal_and_protected_management_in_ap_and_station_modes() {
    for ready in [false, true] {
        let (ip_only, radio, h, _) = setup(ready).await;
        let request = || Request::builder().header("host", "polis-dev.local:8080");
        assert_eq!(
            ip_only
                .oneshot(request().uri("/api/status").body(Body::empty()).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN,
            "hostname access is opt-in"
        );
        let assets = tempfile::tempdir().unwrap();
        std::fs::write(assets.path().join("index.html"), "setup page").unwrap();
        let app = router(
            h.clone(),
            Config {
                web_root: assets.path().into(),
                ap_name: "Setup".into(),
                device_key: Some("test-device-password".into()),
                hostnames: vec!["polis-dev.local".parse().unwrap()],
            },
            "127.0.0.1:8080".into(),
        )
        .unwrap();
        let response = app
            .clone()
            .oneshot(request().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "setup page"
        );
        let response = app
            .clone()
            .oneshot(request().uri("/api/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let status: dto::PortalStatus = json(response).await;
        let body = r#"{"ssid":"Home","security":"Personal"}"#;
        for (origin, csrf, key, expected) in [
            (
                "http://evil.example",
                status.csrf.as_str(),
                "test-device-password",
                StatusCode::FORBIDDEN,
            ),
            (
                "http://127.0.0.1:8080",
                status.csrf.as_str(),
                "test-device-password",
                StatusCode::FORBIDDEN,
            ),
            (
                "http://polis-dev.local:8080",
                "",
                "test-device-password",
                StatusCode::FORBIDDEN,
            ),
            (
                "http://polis-dev.local:8080",
                status.csrf.as_str(),
                "wrong-password",
                StatusCode::UNAUTHORIZED,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    request()
                        .method("POST")
                        .uri("/api/forget")
                        .header("origin", origin)
                        .header("x-wifi-setup-csrf", csrf)
                        .header("authorization", format!("Bearer {key}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected);
        }
        assert!(radio.0.lock().unwrap().2.is_empty());
        // Exercise actual mutations via the hostname in both operating modes.
        let (path, body) = if ready {
            ("/api/forget", body)
        } else {
            ("/api/connect", INPUT)
        };
        let response = app
            .clone()
            .oneshot(
                request()
                    .method("POST")
                    .uri(path)
                    .header("origin", "http://polis-dev.local:8080")
                    .header("x-wifi-setup-csrf", &status.csrf)
                    .header("authorization", "Bearer test-device-password")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            call(&app, "GET", "/api/status", "", "").await.status(),
            StatusCode::OK
        );
        h.shutdown().await.unwrap();
    }
}
