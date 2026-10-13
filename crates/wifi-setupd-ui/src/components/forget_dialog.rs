use crate::state::PortalState;
use dioxus::prelude::*;

#[component]
pub fn ForgetDialog() -> Element {
    let state = use_context::<PortalState>();
    let PortalState {
        mut confirm,
        mut device_key,
        mut message,
        busy,
        offline,
        ..
    } = state;
    // Keep the native dialog mounted so close() restores focus to its opener.
    // showModal() supplies the backdrop, focus containment and inert background.
    use_effect(move || {
        let open = confirm.read().is_some();
        document::eval(&format!(
            "const dialog = document.getElementById('forget-dialog');
             if (dialog) {{ if ({open}) {{ if (!dialog.open) dialog.showModal(); }}
             else if (dialog.open) dialog.close(); }}"
        ));
    });
    let mut close = move || {
        if !*busy.peek() {
            confirm.set(None);
            message.set(String::new());
        }
    };
    rsx! {
        dialog {
            id: "forget-dialog", class: "modal",
            aria_labelledby: "forget-title", aria_describedby: "forget-description",
            oncancel: move |event| { event.prevent_default(); close(); },
            onclick: move |_| close(),
            if let Some(selected) = confirm.read().clone() {
                div { class: "modal-content", onclick: move |event| event.stop_propagation(),
                    h2 { id: "forget-title", "Forget {selected.ssid}?" }
                    p { id: "forget-description", "Its saved credentials will be removed. Deleting the current network disconnects this device." }
                    form { onsubmit: move |event| { event.prevent_default(); state.forget(selected.clone()); },
                        label { r#for: "device-password", "Device password" }
                        input { id: "device-password", r#type: "password", autocomplete: "off", autofocus: true,
                            disabled: *busy.read(), value: "{device_key}",
                            oninput: move |e| { device_key.set(e.value()); message.set(String::new()); }
                        }
                        p { class: "hint", "Use the same password you used to join the setup Wi-Fi." }
                        if *offline.read() { p { class: "notice", role: "status", "The device is currently unreachable. Wait for it to reconnect." } }
                        if !message.read().is_empty() { p { class: "error", role: "alert", "{message}" } }
                        div { class: "modal-actions",
                            button { r#type: "button", class: "secondary", disabled: *busy.read(), onclick: move |_| close(), "Cancel" }
                            button { r#type: "submit", disabled: *busy.read() || *offline.read(),
                                if *busy.read() { "Forgetting…" } else { "Confirm forget" }
                            }
                        }
                    }
                }
            }
        }
    }
}
