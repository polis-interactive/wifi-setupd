use crate::state::PortalState;
use dioxus::prelude::*;
const CSS: Asset = asset!("/assets/style.css");

#[component]
fn ConnectionText(
    ready: bool,
    switching: bool,
    connected: String,
    joining: String,
    setup: String,
) -> Element {
    let text = if switching {
        joining
    } else if ready {
        connected
    } else {
        setup
    };
    rsx! { "{text}" }
}
#[component]
pub fn Shell(children: Element) -> Element {
    let state = use_context::<PortalState>();
    let ready = state.ready();
    let PortalState {
        offline,
        switching,
        message,
        ..
    } = state;
    // Literal props avoid Dioxus 0.7.10 hot-reload collisions between repeated
    // conditions and retain edits to the text for the currently inactive state.
    rsx! {
        document::Title { "wifi-setupd" }
        document::Meta { name: "viewport", content: "width=device-width, initial-scale=1" }
        document::Link { rel: "stylesheet", href: CSS }
        main {
            onpointerdown: move |_| state.dismiss_error(),
            onclick: move |_| state.dismiss_error(),
            onkeydown: move |_| state.dismiss_error(),
            oninput: move |_| state.dismiss_error(),
            onchange: move |_| state.dismiss_error(),
            onfocusin: move |_| state.dismiss_error(),
            onwheel: move |_| state.dismiss_error(),
            header {
                div { class: "brand", span { class: "mark", "◉" } "wifi-setupd" }
                span { class: if ready { "badge connected" } else { "badge" },
                    ConnectionText { ready, switching: *switching.read(), connected: "Connected", joining: "Joining", setup: "Setup" }
                }
            }
            if *offline.read() {
                div { class: "notice", role: "status", "The device is currently unreachable. "
                    if *switching.read() { "It may be joining your Wi-Fi." } else { "Check your phone’s Wi-Fi connection. This page will reconnect automatically." }
                }
            }
            {children}
            if state.confirm.read().is_none() && !message.read().is_empty() { div { class: "notice", role: "alert", "{message}" } }
            footer { "wifi-setupd" span { "•" } "An app by Polis!" }
        }
    }
}
