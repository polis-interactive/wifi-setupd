//! Wi-Fi portal with explicit trusted hosts. Secrets stay in pending requests or iwd.
mod authority;
use crate::{
    provisioner::{CommandError, Handle},
    wifi::{self, Adapter, Credentials},
};
pub use authority::Hostname;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{HeaderMap, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use std::{
    collections::HashMap,
    io::Read,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;
use wifi_setupd_http_api as dto;
use wifi_setupd_zbus_api::ProvisioningState;

pub struct Config {
    pub web_root: PathBuf,
    pub ap_name: String,
    /// Same per-device secret used to provision the AP; None disables deletion.
    pub device_key: Option<String>,
    /// Additional trusted DNS names, served on the same port as the local IP.
    pub hostnames: Vec<Hostname>,
}
struct Pending {
    id: String,
    request: dto::ConnectRequest,
}
struct Portal<A: Adapter + 'static> {
    handle: Handle<A>,
    config: Config,
    csrf: String,
    pending: Mutex<Option<Pending>>,
    begun: Mutex<Option<String>>,
}
fn token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
type Failure = (StatusCode, Json<dto::ApiError>);
fn fail(code: StatusCode, error: &str) -> Failure {
    (
        code,
        Json(dto::ApiError {
            error: error.into(),
        }),
    )
}
fn command(error: CommandError) -> Failure {
    fail(
        StatusCode::CONFLICT,
        match error {
            CommandError::Busy => "A Wi-Fi operation is already running.",
            CommandError::InvalidInput => "Check the network name and password.",
            _ => "Wi-Fi is temporarily unavailable.",
        },
    )
}
fn network(n: wifi::Network) -> dto::Network {
    dto::Network {
        ssid: n.ssid,
        security: format!("{:?}", n.security),
    }
}

/// A router bound to a local IP authority and explicit hostname aliases.
/// AP discovery redirects foreign GETs to the first trusted hostname, or the IP
/// when no hostname is configured.
pub fn router<A: Adapter + 'static>(
    handle: Handle<A>,
    config: Config,
    authority: String,
) -> std::io::Result<Router> {
    let root = config.web_root.clone();
    let authorities = Arc::new(authority::Authorities::new(
        &authority,
        config.hostnames.clone(),
    )?);
    let redirect_authority = authorities.redirect_authority();
    let discovery = handle.clone();
    let portal = Arc::new(Portal {
        handle,
        config,
        csrf: token()?,
        pending: Mutex::new(None),
        begun: Mutex::new(None),
    });
    Ok(Router::new()
        .route("/api/status", get(status::<A>))
        .route("/api/connect", post(reserve::<A>))
        .route("/api/connect/{id}/begin", post(begin::<A>))
        .route("/api/forget", post(forget::<A>))
        .fallback_service(ServeDir::new(root).append_index_html_on_directories(true))
        .layer(DefaultBodyLimit::max(4096))
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let authority = redirect_authority.clone();
            let authorities = authorities.clone();
            let discovery = discovery.clone();
            async move {
                let headers = request.headers();
                let host = headers.get("host").and_then(|v| v.to_str().ok());
                let local = authorities.local_host(headers);
                let probe = matches!(
                    request.uri().path(),
                    "/hotspot-detect.html"
                        | "/library/test/success.html"
                        | "/generate_204"
                        | "/gen_204"
                        | "/connecttest.txt"
                        | "/ncsi.txt"
                        | "/redirect"
                        | "/canonical.html"
                        | "/success.txt"
                );
                // Foreign hosts get only a fixed redirect, never the UI or API.
                // Preserve strict API/mutation checks even while the AP is up.
                let redirect = crate::captive::active(discovery.status())
                    && matches!(*request.method(), Method::GET | Method::HEAD)
                    && host.is_some()
                    && !headers.contains_key("origin")
                    && !request.uri().path().starts_with("/api")
                    && (!local || probe);
                let mut response: Response = if redirect {
                    (
                        StatusCode::FOUND,
                        [("location", format!("http://{authority}/"))],
                    )
                        .into_response()
                } else if !local || !authorities.same_origin(headers) {
                    fail(StatusCode::FORBIDDEN, "Use the device's local address.").into_response()
                } else {
                    next.run(request).await
                };
                response
                    .headers_mut()
                    .insert("cache-control", "no-store".parse().unwrap());
                response
                    .headers_mut()
                    .insert("x-content-type-options", "nosniff".parse().unwrap());
                response
                    .headers_mut()
                    .insert("referrer-policy", "no-referrer".parse().unwrap());
                response
                    .headers_mut()
                    .insert("x-frame-options", "DENY".parse().unwrap());
                response
            }
        }))
        .with_state(portal))
}
fn authorize<A: Adapter + 'static>(p: &Portal<A>, headers: &HeaderMap) -> Result<(), Failure> {
    if headers
        .get("x-wifi-setup-csrf")
        .and_then(|v| v.to_str().ok())
        != Some(p.csrf.as_str())
    {
        return Err(fail(
            StatusCode::FORBIDDEN,
            "Refresh the page before trying again.",
        ));
    }
    Ok(())
}
async fn status<A: Adapter + 'static>(State(p): State<Arc<Portal<A>>>) -> Json<dto::PortalStatus> {
    let saved = p.handle.known_networks().await;
    let (s, connected_ssid) = p.handle.network_status();
    Json(dto::PortalStatus {
        state: format!("{:?}", s.state),
        link: format!("{:?}", s.link),
        connected_ssid,
        backend: format!("{:?}", s.backend),
        error: (s.last_error != wifi_setupd_zbus_api::ProvisioningError::None)
            .then(|| format!("{:?}", s.last_error)),
        ap_name: p.config.ap_name.clone(),
        csrf: p.csrf.clone(),
        networks: p
            .handle
            .cached_networks()
            .into_iter()
            .map(network)
            .collect(),
        saved_available: saved.is_ok(),
        saved: saved
            .unwrap_or_default()
            .into_iter()
            .map(|n| network(n.network))
            .collect(),
        scan_cached: true,
    })
}
async fn reserve<A: Adapter + 'static>(
    State(p): State<Arc<Portal<A>>>,
    headers: HeaderMap,
    body: Result<Json<dto::ConnectRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<(StatusCode, Json<dto::Accepted>), Failure> {
    authorize(&p, &headers)?;
    let Json(request) = body.map_err(|_| fail(StatusCode::BAD_REQUEST, "Invalid request."))?;
    if request.ssid.is_empty()
        || request.ssid.len() > 32
        || request.ssid.contains('\0')
        || (!request.password.is_empty()
            && (!(8..=63).contains(&request.password.len()) || !request.password.is_ascii()))
    {
        return Err(fail(
            StatusCode::BAD_REQUEST,
            "Use a network name of 1–32 bytes and a password of 8–63 ASCII characters (or empty for open Wi-Fi).",
        ));
    }
    let mut pending = p.pending.lock().await;
    if pending.is_some() || p.handle.status().state != ProvisioningState::AwaitingCredentials {
        return Err(command(CommandError::Busy));
    }
    let id = token().map_err(|_| fail(StatusCode::INTERNAL_SERVER_ERROR, "Please retry."))?;
    *pending = Some(Pending {
        id: id.clone(),
        request,
    });
    let weak = Arc::downgrade(&p);
    let expiry_id = id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        if let Some(p) = weak.upgrade() {
            let mut pending = p.pending.lock().await;
            if pending.as_ref().is_some_and(|v| v.id == expiry_id) {
                *pending = None;
            }
        }
    });
    // The browser must receive this response before it can request AP teardown.
    Ok((StatusCode::ACCEPTED, Json(dto::Accepted { attempt: id })))
}
async fn begin<A: Adapter + 'static>(
    State(p): State<Arc<Portal<A>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    authorize(&p, &headers)?;
    let mut begun = p.begun.lock().await;
    if begun.as_ref() == Some(&id) {
        return Ok(StatusCode::ACCEPTED);
    }
    let mut pending = p.pending.lock().await;
    if !pending.as_ref().is_some_and(|v| v.id == id) {
        return Err(fail(
            StatusCode::GONE,
            "Request expired. Enter the password and try again.",
        ));
    }
    let request = pending.take().unwrap().request;
    p.handle
        .submit(
            Credentials {
                ssid: request.ssid,
                password: request.password,
            },
            request.replace,
        )
        .await
        .map_err(command)?;
    *begun = Some(id);
    Ok(StatusCode::ACCEPTED)
}
async fn forget<A: Adapter + 'static>(
    State(p): State<Arc<Portal<A>>>,
    headers: HeaderMap,
    body: Result<Json<dto::Network>, axum::extract::rejection::JsonRejection>,
) -> Result<StatusCode, Failure> {
    authorize(&p, &headers)?;
    if !p.config.device_key.as_ref().is_some_and(|key| {
        headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.strip_prefix("Bearer ") == Some(key.as_str()))
    }) {
        return Err(fail(
            StatusCode::UNAUTHORIZED,
            "Enter the device password used to join the setup Wi-Fi.",
        ));
    }
    let Json(selected) = body.map_err(|_| fail(StatusCode::BAD_REQUEST, "Invalid network."))?;
    let known = p.handle.known_networks().await.map_err(command)?;
    let selected = known
        .into_iter()
        .find(|n| network(n.network.clone()) == selected)
        .ok_or_else(|| {
            fail(
                StatusCode::NOT_FOUND,
                "That saved network is no longer present.",
            )
        })?;
    p.handle.forget(selected.network).await.map_err(command)?;
    Ok(StatusCode::ACCEPTED)
}

/// Rebind as the named Wi-Fi interface changes address. Ethernet is never selected.
pub async fn serve_interface<A: Adapter + 'static>(
    handle: Handle<A>,
    interface: String,
    config: Config,
    port: u16,
) {
    let mut tasks = tokio::task::JoinSet::new();
    let mut listeners: HashMap<Ipv4Addr, tokio::task::AbortHandle> = HashMap::new();
    let mut dns: HashMap<Ipv4Addr, tokio::task::AbortHandle> = HashMap::new();
    let mut status = handle.subscribe();
    loop {
        let addresses: Vec<_> = nix::ifaddrs::getifaddrs()
            .into_iter()
            .flatten()
            .filter(|a| a.interface_name == interface)
            .filter_map(|a| a.address.and_then(|a| a.as_sockaddr_in().map(|a| a.ip())))
            .filter(|a| !a.is_unspecified() && !a.is_loopback() && !a.is_link_local())
            .collect();
        listeners.retain(|ip, task| {
            if !addresses.contains(ip) || task.is_finished() {
                task.abort();
                false
            } else {
                true
            }
        });
        let captive = port == 80 && crate::captive::active(handle.status());
        dns.retain(|ip, task| {
            if !captive || !addresses.contains(ip) || task.is_finished() {
                task.abort();
                false
            } else {
                true
            }
        });
        for &ip in &addresses {
            if !captive || dns.contains_key(&ip) {
                continue;
            }
            let sockets = async {
                let udp = tokio::net::UdpSocket::bind((ip, 53)).await?;
                let tcp = tokio::net::TcpListener::bind((ip, 53)).await?;
                Ok::<_, std::io::Error>((udp, tcp))
            }
            .await;
            match sockets {
                Ok((udp, tcp)) => {
                    let status = handle.subscribe();
                    eprintln!("wifi-setupd: captive DNS listening on {ip}:53");
                    dns.insert(
                        ip,
                        tasks.spawn(async move {
                            if let Err(error) = crate::captive::serve(udp, tcp, ip, status).await {
                                eprintln!("wifi-setupd: captive DNS stopped: {error}");
                            }
                        }),
                    );
                }
                Err(error) => eprintln!("wifi-setupd: cannot bind captive DNS on {ip}:53: {error}"),
            }
        }
        for ip in addresses {
            if listeners.contains_key(&ip) {
                continue;
            }
            let address = SocketAddr::from((ip, port));
            let Ok(listener) = tokio::net::TcpListener::bind(address).await else {
                continue;
            };
            let authority = if port == 80 {
                ip.to_string()
            } else {
                address.to_string()
            };
            let Ok(router) = router(
                handle.clone(),
                Config {
                    web_root: config.web_root.clone(),
                    ap_name: config.ap_name.clone(),
                    device_key: config.device_key.clone(),
                    hostnames: config.hostnames.clone(),
                },
                authority,
            ) else {
                continue;
            };
            eprintln!("wifi-setupd: portal listening at http://{address}");
            listeners.insert(
                ip,
                tasks.spawn(async move {
                    let _ = axum::serve(listener, router).await;
                }),
            );
        }
        while tasks.try_join_next().is_some() {}
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
            changed = status.changed() => { if changed.is_err() { break; } },
        }
    }
}

#[cfg(test)]
mod tests;
