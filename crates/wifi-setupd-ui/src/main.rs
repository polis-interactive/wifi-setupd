mod components;
mod layout;
mod pages;
mod state;

use dioxus::prelude::*;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let state = state::use_portal_state();
    use_context_provider(|| state);
    rsx! { layout::Shell { pages::PortalPage {} } }
}
