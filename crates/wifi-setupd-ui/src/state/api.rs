use gloo_net::http::Request;
use wifi_setupd_http_api::{ApiError, PortalStatus};
pub(super) enum PostError {
    Rejected(String),
    Uncertain(String),
}
impl std::fmt::Display for PostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(message) | Self::Uncertain(message) => f.write_str(message),
        }
    }
}
async fn bounded<T>(
    future: impl std::future::Future<Output = Result<T, gloo_net::Error>>,
) -> Result<T, String> {
    let timeout = gloo_timers::future::TimeoutFuture::new(10000);
    match futures_util::future::select(Box::pin(future), Box::pin(timeout)).await {
        futures_util::future::Either::Left((result, _)) => {
            result.map_err(|_| "Connection interrupted. The device may still be working.".into())
        }
        futures_util::future::Either::Right(_) => {
            Err("Device unreachable. The device may still be working.".into())
        }
    }
}
pub(super) async fn get_status() -> Result<PortalStatus, String> {
    bounded(Request::get("/api/status").send())
        .await?
        .json()
        .await
        .map_err(|_| "Device unavailable".to_string())
}
pub(super) async fn post<T: serde::Serialize>(
    path: &str,
    body: &T,
    csrf: &str,
    key: &str,
) -> Result<gloo_net::http::Response, PostError> {
    let request = Request::post(path).header("X-Wifi-Setup-CSRF", csrf);
    let request = if key.is_empty() {
        request
    } else {
        request.header("Authorization", &format!("Bearer {key}"))
    };
    let request = request
        .json(body)
        .map_err(|_| PostError::Rejected("Could not prepare request.".to_string()))?;
    let response = bounded(request.send())
        .await
        .map_err(PostError::Uncertain)?;
    if !response.ok() {
        let message = response
            .json::<ApiError>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| "Device unavailable. Please retry.".into());
        return Err(if response.status() < 500 {
            PostError::Rejected(message)
        } else {
            PostError::Uncertain(message)
        });
    }
    Ok(response)
}
