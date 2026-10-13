use crate::{components::SavedNetworks, state::PortalState};
use dioxus::prelude::*;

#[component]
fn ConnectedContent(active: bool, title: String, ssid: Option<String>) -> Element {
    if !active {
        return rsx! {};
    }
    rsx! {
        section { class: "intro",
            h1 {
                span { class: "connection-heading", "{title}" }
                if let Some(ssid) = ssid { span { class: "connected-ssid", "{ssid}" } }
            }
        }
        SavedNetworks {}
    }
}

#[component]
pub fn Connected() -> Element {
    let state = use_context::<PortalState>();
    let active = state.ready() && !*state.switching.read();
    let ssid = state
        .status
        .read()
        .as_ref()
        .and_then(|status| status.connected_ssid.clone());
    // Keep the literal prop mounted so Dioxus can hot reload this heading even
    // before the connected view is first shown.
    rsx! { ConnectedContent { active, title: "Connected to:", ssid } }
}
