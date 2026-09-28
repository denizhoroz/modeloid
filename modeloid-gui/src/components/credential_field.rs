use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::settings::{
    remove_roboflow_key, roboflow_key_status, save_roboflow_key, validate_api_key, KeyStatus,
};
use crate::components::{Icon, IconName};

const ROBOFLOW_KEY_DOCS: &str =
    "https://docs.roboflow.com/developer/authentication/find-your-roboflow-api-key";

/// Roboflow API key entry: save, replace or remove the key stored on the server.
#[component]
pub fn RoboflowKeyField() -> Element {
    let mut status = use_signal(|| None::<KeyStatus>);
    let mut key = use_signal(String::new);
    let mut reveal = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<&'static str>);

    use_future(move || async move {
        match roboflow_key_status().await {
            Ok(s) => status.set(Some(s)),
            Err(e) => error.set(Some(error_message(&e))),
        }
    });

    let save = move |e: FormEvent| async move {
        e.prevent_default();
        notice.set(None);
        if let Err(message) = validate_api_key(&key.read()) {
            error.set(Some(message));
            return;
        }
        busy.set(true);
        match save_roboflow_key(key()).await {
            Ok(s) => {
                status.set(Some(s));
                key.set(String::new());
                reveal.set(false);
                error.set(None);
                notice.set(Some("Key saved."));
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        busy.set(false);
    };

    let remove = move |_| async move {
        notice.set(None);
        busy.set(true);
        match remove_roboflow_key().await {
            Ok(s) => {
                status.set(Some(s));
                error.set(None);
                notice.set(Some("Key removed."));
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        busy.set(false);
    };

    let current = status();
    let saved_hint = current.as_ref().and_then(|s| s.saved_hint.clone());

    rsx! {
        form { class: "field", novalidate: true, onsubmit: save,
            div { class: "field-row",
                label { class: "field-label", r#for: "roboflow-key", "Roboflow API key" }
                match &current {
                    Some(KeyStatus { saved_hint: Some(hint), .. }) => rsx! {
                        span { class: "badge badge-ok", "Saved · ends in {hint}" }
                    },
                    Some(KeyStatus { from_env: true, .. }) => rsx! {
                        span { class: "badge", "Using ROBOFLOW_API_KEY env var" }
                    },
                    Some(_) => rsx! { span { class: "badge badge-muted", "Not set" } },
                    None => rsx! {},
                }
            }

            div { class: "input-row",
                div { class: "input-group",
                    input {
                        id: "roboflow-key",
                        class: "input",
                        r#type: if reveal() { "text" } else { "password" },
                        autocomplete: "off",
                        spellcheck: "false",
                        placeholder: if saved_hint.is_some() { "Paste a new key to replace it" } else { "Paste your API key" },
                        aria_invalid: if error().is_some() { "true" },
                        aria_describedby: "roboflow-key-help",
                        value: "{key}",
                        oninput: move |e| {
                            key.set(e.value());
                            error.set(None);
                        },
                    }
                    button {
                        class: "input-addon",
                        r#type: "button",
                        aria_label: if reveal() { "Hide key" } else { "Show key" },
                        aria_pressed: if reveal() { "true" } else { "false" },
                        onclick: move |_| reveal.toggle(),
                        Icon { name: if reveal() { IconName::EyeOff } else { IconName::Eye }, size: 14 }
                    }
                }
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: busy() || key.read().trim().is_empty(),
                    if saved_hint.is_some() { "Replace" } else { "Save" }
                }
                if saved_hint.is_some() {
                    button {
                        class: "button button-danger",
                        r#type: "button",
                        disabled: busy(),
                        onclick: remove,
                        "Remove"
                    }
                }
            }

            div { id: "roboflow-key-help", aria_live: "polite",
                if let Some(message) = error() {
                    p { class: "field-error", role: "alert", "{message}" }
                } else if let Some(message) = notice() {
                    p { class: "field-success", "{message}" }
                }
                p { class: "field-hint",
                    "Used to download datasets from Roboflow. "
                    a { href: ROBOFLOW_KEY_DOCS, target: "_blank", rel: "noopener noreferrer", "Where to find it" }
                }
                if let Some(s) = &current {
                    p { class: "field-hint", "Stored on this computer in "
                        code { "{s.settings_path}" }
                    }
                }
            }
        }
    }
}
