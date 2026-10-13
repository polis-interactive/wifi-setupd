use dioxus::prelude::*;
use wifi_setupd_http_api::Network;

/// Editable combobox with manual selection, following the WAI-ARIA APG pattern:
/// https://www.w3.org/WAI/ARIA/apg/patterns/combobox/examples/combobox-autocomplete-list/
/// Typing an unlisted SSID is sufficient; selecting a suggestion is optional.
#[component]
pub fn Networks(mut value: Signal<String>, networks: Vec<Network>, disabled: bool) -> Element {
    let mut expanded = use_signal(|| false);
    let mut active = use_signal(|| None::<usize>);
    let mut show_all = use_signal(|| false);
    let query = value.read().clone();
    let mut names = Vec::new();
    for network in networks {
        if (*show_all.read() || network.ssid.to_lowercase().contains(&query.to_lowercase()))
            && !names.contains(&network.ssid)
        {
            names.push(network.ssid);
        }
    }
    let custom = !query.is_empty() && !names.contains(&query);
    if custom {
        names.push(query.clone());
    }
    let open = *expanded.read() && !disabled && !names.is_empty();
    let selected = active.read().filter(|index| *index < names.len());
    let active_id = if open {
        selected.map(|index| format!("network-option-{index}"))
    } else {
        None
    };
    use_effect(move || {
        if *expanded.read()
            && let Some(index) = *active.read()
        {
            // Only a numeric index is interpolated, never an SSID.
            document::eval(&format!(
                "document.getElementById('network-option-{index}')?.scrollIntoView({{block:'nearest'}})"
            ));
        }
    });
    let keyboard_names = names.clone();
    rsx! {
        label { r#for: "networks", "Networks" }
        div { class: "combobox",
            div { class: "combobox-input",
                input {
                    id: "networks", name: "ssid", role: "combobox", r#type: "text",
                    autocomplete: "off", autocapitalize: "none", spellcheck: "false",
                    required: true, maxlength: "32", disabled, value: "{value}",
                    placeholder: "Select or type a network",
                    aria_autocomplete: "list", aria_controls: "network-options",
                    aria_expanded: open, aria_activedescendant: active_id,
                    onfocus: move |_| { expanded.set(true); show_all.set(true); active.set(None); },
                    onblur: move |_| { expanded.set(false); active.set(None); },
                    oninput: move |event| { value.set(event.value()); show_all.set(false); expanded.set(true); active.set(None); },
                    onkeydown: move |event: KeyboardEvent| {
                        let count = keyboard_names.len();
                        match event.key() {
                            Key::ArrowDown | Key::ArrowUp if count > 0 => {
                                event.prevent_default();
                                let next = match (event.key(), selected.filter(|_| open)) {
                                    (Key::ArrowDown, Some(index)) => (index + 1) % count,
                                    (Key::ArrowUp, Some(index)) => (index + count - 1) % count,
                                    (Key::ArrowUp, None) => count - 1,
                                    _ => 0,
                                };
                                expanded.set(true); active.set(Some(next));
                            }
                            Key::Enter if open => {
                                event.prevent_default();
                                if let Some(index) = selected { value.set(keyboard_names[index].clone()); }
                                expanded.set(false); active.set(None);
                            }
                            Key::Escape => { event.prevent_default(); expanded.set(false); active.set(None); }
                            Key::ArrowLeft | Key::ArrowRight | Key::Home | Key::End => active.set(None),
                            _ => {}
                        }
                    },
                }
                button {
                    r#type: "button", class: "combobox-toggle", tabindex: "-1", disabled,
                    aria_label: "Show networks", aria_controls: "network-options", aria_expanded: open,
                    onmousedown: move |event| event.prevent_default(),
                    onclick: move |_| {
                        if !open { document::eval("document.getElementById('networks')?.focus()"); }
                        expanded.set(!open); show_all.set(true); active.set(None);
                    },
                    "▾"
                }
            }
            ul { id: "network-options", class: "network-options", role: "listbox", aria_label: "Networks", hidden: !open,
                for (index, name) in names.iter().enumerate() {
                    li {
                        id: "network-option-{index}", role: "option", aria_selected: selected == Some(index),
                        onmousedown: move |event| event.prevent_default(),
                        onclick: { let name = name.clone(); move |_| { value.set(name.clone()); expanded.set(false); active.set(None); } },
                        if custom && index + 1 == names.len() { "Use “{name}”" } else { "{name}" }
                    }
                }
            }
        }
    }
}
