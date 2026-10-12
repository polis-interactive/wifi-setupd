use dioxus::prelude::*;
use gloo_net::http::Request;
use wifi_setupd_http_api::{Accepted, ApiError, ConnectRequest, Network, PortalStatus};
const CSS: Asset = asset!("/assets/style.css");
fn main() {
    dioxus::launch(App);
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
async fn get_status() -> Result<PortalStatus, String> {
    bounded(Request::get("/api/status").send())
        .await?
        .json()
        .await
        .map_err(|_| "Device unavailable".to_string())
}
async fn post<T: serde::Serialize>(
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
fn friendly(error: &str) -> &str {
    match error {
        "AuthenticationFailed" => "The network rejected the password. Check it and try again.",
        "NetworkUnavailable" => "That network could not be found. Check the name and try again.",
        "CredentialConflict" => {
            "This network is already saved. Choose to replace its saved password."
        }
        "TimedOut" => "The connection took too long. Check the network and try again.",
        "AddressUnavailable" => {
            "Wi-Fi connected but did not receive an IP address. Check the router and retry."
        }
        _ => "The connection could not be completed. Check the network and password, then retry.",
    }
}
#[component]
fn App() -> Element {
    let mut status = use_signal(|| None::<PortalStatus>);
    let mut offline = use_signal(|| false);
    let mut ssid = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut open_network = use_signal(|| false);
    let mut show_password = use_signal(|| false);
    let mut replace = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut switching = use_signal(|| false);
    let mut message = use_signal(String::new);
    let mut key = use_signal(String::new);
    let mut confirm = use_signal(|| None::<Network>);
    use_future(move || async move {
        loop {
            match get_status().await {
                Ok(mut next) => {
                    if !next.saved_available
                        && let Some(previous) = status.peek().as_ref()
                    {
                        next.saved = previous.saved.clone();
                    }
                    if next.link == "Ready"
                        || (next.state == "AwaitingCredentials" && next.error.is_some())
                    {
                        switching.set(false);
                    }
                    status.set(Some(next));
                    offline.set(false);
                }
                Err(_) => offline.set(true),
            }
            gloo_timers::future::TimeoutFuture::new(2000).await;
        }
    });
    let snapshot = status.read().clone();
    let ready = snapshot.as_ref().is_some_and(|s| s.link == "Ready");
    let awaiting = snapshot
        .as_ref()
        .is_some_and(|s| s.state == "AwaitingCredentials");
    let ap_name = snapshot
        .as_ref()
        .map(|s| s.ap_name.clone())
        .unwrap_or_else(|| "the setup network".into());
    let saved_match = snapshot
        .as_ref()
        .is_some_and(|s| s.saved.iter().any(|n| n.ssid == *ssid.read()));
    let disabled = *busy.read() || *switching.read() || !awaiting || *offline.read();
    let submit = move |event: FormEvent| {
        event.prevent_default();
        if disabled {
            return;
        }
        let Some(s) = status.peek().clone() else {
            return;
        };
        busy.set(true);
        message.set(String::new());
        let request = ConnectRequest {
            ssid: ssid.peek().clone(),
            password: if *open_network.peek() {
                String::new()
            } else {
                password.peek().clone()
            },
            replace: *replace.peek(),
        };
        spawn(async move {
            match post("/api/connect", &request, &s.csrf, "").await {
                Ok(response) => match response.json::<Accepted>().await {
                    Ok(accepted) => {
                        password.set(String::new());
                        switching.set(true);
                        // Acceptance arrived while the AP was still available.
                        if let Err(error) = post(
                            &format!("/api/connect/{}/begin", accepted.attempt),
                            &(),
                            &s.csrf,
                            "",
                        )
                        .await
                        {
                            message.set(error);
                        }
                    }
                    Err(_) => message
                        .set("Could not read acceptance. Wait 30 seconds, then retry.".into()),
                },
                Err(error) => message.set(error),
            }
            busy.set(false);
        });
    };
    rsx! {
        document::Title { "Wi-Fi setup" }
        document::Meta { name: "viewport", content: "width=device-width, initial-scale=1" }
        document::Link { rel: "stylesheet", href: CSS }
        main {
            header {
                div { class: "brand", span { class: "mark", "◉" } "DEVICE / WI-FI" }
                span { class: if ready { "badge connected" } else { "badge" }, if ready { "Connected" } else { "Setup" } }
            }
            section { class: "intro",
                p { class: "eyebrow", "A LITTLE SETUP. THEN YOU’RE IN." }
                h1 { if ready { "You’re connected." } else { "Make yourself\nat home." } }
                p { class: "lede", if ready { "Your device is on Wi-Fi. Manage the networks it remembers below." } else { "Choose your Wi-Fi network to bring this device online. No internet connection is required for setup." } }
            }
            if *offline.read() {
                div { class: "notice", role: "status", "The device is currently unreachable. "
                    if *switching.read() { "It may be joining your Wi-Fi." } else { "Check your phone’s Wi-Fi connection. This page will reconnect automatically." }
                }
            }
            if *switching.read() {
                section { class: "card", role: "status",
                    h2 { "Joining your network…" }
                    p { "The setup Wi-Fi will disconnect while the device connects. Join your normal Wi-Fi afterward and open the device’s new local IP address." }
                    p { "If the connection fails, reconnect to {ap_name} and reopen this page to retry." }
                    button { class: "secondary", onclick: move |_| { switching.set(false); }, "I’m back on the setup network" }
                }
            } else if !ready {
                section { class: "card",
                    h2 { "Connect to Wi-Fi" }
                    if let Some(s) = &snapshot {
                        if let Some(error) = &s.error { div { class: "error", role: "alert", "{friendly(error)}" } }
                        if !s.networks.is_empty() {
                            label { r#for: "networks", "Nearby networks" }
                            select { id: "networks", disabled, value: "", onchange: move |e| { ssid.set(e.value()); replace.set(false); },
                                option { value: "", disabled: true, "Choose a network" }
                                for network in &s.networks { option { value: "{network.ssid}", "{network.ssid}" } }
                            }
                            p { class: "hint", "Found before setup started. You can also enter a network name below." }
                        }
                    }
                    form { onsubmit: submit, method: "post", action: "/api/connect",
                        label { r#for: "ssid", "Network name" }
                        input { id: "ssid", name: "ssid", autocomplete: "off", required: true, maxlength: "32", disabled, value: "{ssid}", oninput: move |e| { ssid.set(e.value()); replace.set(false); }, placeholder: "Your Wi-Fi name" }
                        label { class: "check", input { r#type: "checkbox", checked: *open_network.read(), disabled, onchange: move |e| { open_network.set(e.checked()); password.set(String::new()); } } "This is an open network (no password)" }
                        if !*open_network.read() {
                            label { r#for: "password", "Wi-Fi password" }
                            input { id: "password", name: "password", r#type: if *show_password.read() { "text" } else { "password" }, autocomplete: "off", required: true, minlength: "8", maxlength: "63", disabled, value: "{password}", oninput: move |e| password.set(e.value()) }
                            label { class: "check", input { r#type: "checkbox", checked: *show_password.read(), onchange: move |e| show_password.set(e.checked()) } "Show password" }
                        }
                        label { class: "check", input { r#type: "checkbox", required: saved_match, checked: *replace.read(), disabled, onchange: move |e| replace.set(e.checked()) } "Replace any saved credentials for this network" }
                        button { r#type: "submit", disabled, if *busy.read() { "Sending…" } else { "Connect device →" } }
                    }
                    if !awaiting && !*offline.read() { p { class: "hint", "Preparing Wi-Fi… This page updates automatically." } }
                    p { class: "hint", "The network is saved on your device. This page does not store your password." }
                }
            } else {
                section { class: "card",
                    h2 { "Saved networks" }
                    p { class: "hint", "Forget networks you no longer use. Deleting the current network may disconnect this device." }
                    label { r#for: "management-key", "Device management key" }
                    input { id: "management-key", r#type: "password", autocomplete: "off", value: "{key}", oninput: move |e| key.set(e.value()), placeholder: "Required to forget a network" }
                    if let Some(s) = &snapshot {
                        if s.saved.is_empty() { p { "No saved networks to display." } }
                        for network in &s.saved {
                            div { class: "network", span { "{network.ssid}" }
                                button { class: "secondary", disabled: *busy.read() || !s.saved_available, onclick: { let n = network.clone(); move |_| confirm.set(Some(n.clone())) }, "Forget" }
                            }
                        }
                    }
                    if let Some(selected) = confirm.read().clone() {
                        div { class: "notice",
                            p { "Forget {selected.ssid}? Its saved credentials will be removed." }
                            button { disabled: *busy.read(), onclick: move |_| {
                                let Some(s) = status.peek().clone() else { return; };
                                let selected = selected.clone(); let secret = key.peek().clone(); busy.set(true);
                                spawn(async move {
                                    match post("/api/forget", &selected, &s.csrf, &secret).await {
                                        Ok(_) => { confirm.set(None); message.set("Network deletion requested.".into()); }
                                        Err(error) => message.set(error),
                                    }
                                    busy.set(false);
                                });
                            }, "Confirm forget" }
                            button { class: "secondary", onclick: move |_| confirm.set(None), "Cancel" }
                        }
                    }
                }
            }
            if !message.read().is_empty() { div { class: "notice", role: "alert", "{message}" } }
            footer { "LOCAL SETUP" span { "•" } "Your network. Your device." }
        }
    }
}
