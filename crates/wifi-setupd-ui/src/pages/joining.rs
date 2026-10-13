use crate::state::PortalState;
use dioxus::prelude::*;

#[component]
pub fn Joining() -> Element {
    let mut state = use_context::<PortalState>();
    let ap_name = state
        .status
        .read()
        .as_ref()
        .map(|s| s.ap_name.clone())
        .unwrap_or_else(|| "the setup network".into());
    rsx! {
        section { class: "card", role: "status",
            h2 { "Joining your network…" }
            p { "The setup Wi-Fi will disconnect while the device connects. Join your normal Wi-Fi afterward and open the device’s new local IP address." }
            p { "If the connection fails, reconnect to {ap_name} and reopen this page to retry." }
            button { class: "secondary", onclick: move |_| { state.switching.set(false); }, "I’m back on the setup network" }
        }
    }
}
