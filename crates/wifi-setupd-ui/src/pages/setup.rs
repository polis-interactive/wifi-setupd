use crate::{
    components::Networks,
    state::{PortalState, friendly},
};
use dioxus::prelude::*;

#[component]
pub fn Setup() -> Element {
    let state = use_context::<PortalState>();
    let snapshot = state.status.read().clone();
    let disabled = state.disabled();
    let awaiting = state.awaiting();
    let PortalState {
        ssid,
        mut password,
        mut open_network,
        mut show_password,
        busy,
        offline,
        ..
    } = state;
    rsx! {
        section { class: "card",
            h2 { "Connect to Wi-Fi" }
            form { onsubmit: move |event| { event.prevent_default(); state.submit(); }, method: "post", action: "/api/connect",
                Networks { value: ssid, networks: snapshot.as_ref().map(|s| s.networks.clone()).unwrap_or_default(), disabled }
                label { class: "check", input { r#type: "checkbox", checked: *open_network.read(), disabled, onchange: move |e| { open_network.set(e.checked()); password.set(String::new()); } } "This is an open network (no password)" }
                if !*open_network.read() {
                    label { r#for: "password", "Wi-Fi password" }
                    input { id: "password", name: "password", r#type: if *show_password.read() { "text" } else { "password" }, autocomplete: "off", required: true, minlength: "8", maxlength: "63", disabled, value: "{password}", oninput: move |e| password.set(e.value()) }
                    label { class: "check", input { r#type: "checkbox", checked: *show_password.read(), onchange: move |e| show_password.set(e.checked()) } "Show password" }
                }
                button { r#type: "submit", disabled, if *busy.read() { "Sending…" } else { "Connect device →" } }
            }
            if let Some(error) = state.error.read().visible() {
                div { class: "error", role: "alert", "{friendly(error)}" }
            }
            if !awaiting && !*offline.read() { p { class: "hint", "Preparing Wi-Fi… This page updates automatically." } }
            p { class: "hint", "The network is saved on your device. This page does not store your password." }
        }
    }
}
