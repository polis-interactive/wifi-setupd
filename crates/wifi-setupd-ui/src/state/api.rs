use gloo_net::http::Request;
use wifi_setupd_http_api::{ApiError, PortalStatus};
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
) -> Result<gloo_net::http::Response, String> {
    let request = Request::post(path).header("X-Wifi-Setup-CSRF", csrf);
    let request = if key.is_empty() {
        request
    } else {
        request.header("Authorization", &format!("Bearer {key}"))
    };
    let request = request
        .json(body)
        .map_err(|_| "Could not prepare request.".to_string())?;
    let response = bounded(request.send()).await?;
    if !response.ok() {
        return Err(response
            .json::<ApiError>()
            .await
            .map(|e| e.error)
            .unwrap_or_else(|_| "Device unavailable. Please retry.".into()));
    }
    Ok(response)
}
