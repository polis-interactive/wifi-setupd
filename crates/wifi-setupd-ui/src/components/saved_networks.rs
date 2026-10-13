use crate::{components::ForgetDialog, state::PortalState};
use dioxus::prelude::*;

#[component]
pub fn SavedNetworks() -> Element {
    let state = use_context::<PortalState>();
    let mut snapshot = state.status.read().clone();
    let forgetting = state.forgetting.read();
    if let Some(snapshot) = &mut snapshot {
        snapshot
            .saved
            .retain(|network| forgetting.as_ref() != Some(network));
    }
    let PortalState {
        mut confirm,
        mut message,
        busy,
        ..
    } = state;
    rsx! {
        section { class: "card",
            h2 { "Saved networks" }
            p { class: "hint", "Forget networks you no longer use. Deleting the current network disconnects this device." }
            if let Some(s) = &snapshot {
                if s.saved.is_empty() { p { "No saved networks to display." } }
                for network in &s.saved {
                    div { class: "network", span { "{network.ssid}" }
                        button { class: "secondary", disabled: *busy.read() || forgetting.is_some() || !s.saved_available, onclick: { let n = network.clone(); move |_| { message.set(String::new()); confirm.set(Some(n.clone())); } }, "Forget" }
                    }
                }
            }
            ForgetDialog {}
        }
    }
}
