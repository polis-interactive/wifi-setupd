use crate::state::PortalState;
use dioxus::prelude::*;

#[component]
pub fn Disconnected() -> Element {
    let state = use_context::<PortalState>();
    let ap_name = state
        .status
        .read()
        .as_ref()
        .map(|s| s.ap_name.clone())
        .unwrap_or_else(|| "the setup network".into());
    rsx! {
        section { class: "card", role: "status",
            h2 { "Reconnect to setup Wi-Fi" }
            p { "Forgetting the current network disconnects this device. This page may stop responding." }
            p { "Open your phone’s Wi-Fi settings and join {ap_name} when it appears. The setup page will let you choose another network." }
            p { class: "hint", "Use the same device password to join the setup network." }
        }
    }
}
