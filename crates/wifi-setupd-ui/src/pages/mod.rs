mod connected;
mod joining;
mod setup;
use crate::state::PortalState;
use dioxus::prelude::*;

#[component]
pub fn PortalPage() -> Element {
    let state = use_context::<PortalState>();
    rsx! {
        if *state.switching.read() { joining::Joining {} }
        else if !state.ready() { setup::Setup {} }
        connected::Connected {}
    }
}
