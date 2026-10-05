use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::settings::{
    credential_status, remove_credential, save_credential, validate_credential, KeyStatus, ServiceId,
};
use crate::components::{Icon, IconName, InfoTip};

/// What the credential is for (shown in the label's tip) and its docs link.
fn help(service: ServiceId) -> (&'static str, &'static str) {
    match service {
        ServiceId::Roboflow => (
            "Used to download datasets from Roboflow.",
            "https://docs.roboflow.com/developer/authentication/find-your-roboflow-api-key",
        ),
        ServiceId::Kaggle => (
            "Paste an API token from Kaggle's settings page (starts with KGAT_). A legacy username:key or kaggle.json works too. Without it, KAGGLE_API_TOKEN or ~/.kaggle (access_token, kaggle.json) is used if present.",
            "https://www.kaggle.com/docs/api#authentication",
        ),
        ServiceId::HuggingFace => (
            "Only needed for private or gated datasets; public ones download without it.",
            "https://huggingface.co/docs/hub/security-tokens",
        ),
    }
}

/// Credential entry for one service: save, replace or remove what's stored on the server.
#[component]
pub fn CredentialField(service: ServiceId) -> Element {
    let mut status = use_signal(|| None::<KeyStatus>);
    let mut value = use_signal(String::new);
    let mut reveal = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut notice = use_signal(|| None::<&'static str>);

    use_future(move || async move {
        match credential_status(service).await {
            Ok(s) => status.set(Some(s)),
            Err(e) => error.set(Some(error_message(&e))),
        }
    });

    let save = move |e: FormEvent| async move {
        e.prevent_default();
        notice.set(None);
        if let Err(message) = validate_credential(service, &value.read()) {
            error.set(Some(message));
            return;
        }
        busy.set(true);
        match save_credential(service, value()).await {
            Ok(s) => {
                status.set(Some(s));
                value.set(String::new());
                reveal.set(false);
                error.set(None);
                notice.set(Some("Saved."));
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        busy.set(false);
    };

    let remove = move |_| async move {
        notice.set(None);
        busy.set(true);
        match remove_credential(service).await {
            Ok(s) => {
                status.set(Some(s));
                error.set(None);
                notice.set(Some("Removed."));
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        busy.set(false);
    };

    let current = status();
    let saved = current.as_ref().is_some_and(|s| s.saved_hint.is_some());
    let slug = service.slug();
    let (help_text, docs) = help(service);

    rsx! {
        form { class: "field", novalidate: true, onsubmit: save,
            div { class: "field-row",
                label { class: "field-label", r#for: "{slug}-credential",
                    "{service.credential()} "
                    InfoTip { label: "About the {service.label()} credential", text: help_text }
                }
                match &current {
                    Some(KeyStatus { saved_hint: Some(_), .. }) => rsx! {
                        span { class: "badge badge-ok", "Saved" }
                    },
                    Some(KeyStatus { fallback: Some(source), .. }) => rsx! {
                        span { class: "badge", title: "{source}", "Using {source}" }
                    },
                    Some(_) => rsx! { span { class: "badge badge-muted", "Not set" } },
                    None => rsx! {},
                }
            }

            div { class: "input-row",
                div { class: "input-group",
                    input {
                        id: "{slug}-credential",
                        class: "input",
                        r#type: if reveal() { "text" } else { "password" },
                        autocomplete: "off",
                        spellcheck: "false",
                        placeholder: if saved { "Paste new credentials to replace them" } else if service == ServiceId::Kaggle { "KGAT_… API token" } else { "Paste your key" },
                        aria_invalid: if error().is_some() { "true" },
                        aria_describedby: "{slug}-credential-help",
                        value: "{value}",
                        oninput: move |e| {
                            value.set(e.value());
                            error.set(None);
                        },
                    }
                    button {
                        class: "input-addon",
                        r#type: "button",
                        aria_label: if reveal() { "Hide" } else { "Show" },
                        aria_pressed: if reveal() { "true" } else { "false" },
                        onclick: move |_| reveal.toggle(),
                        Icon { name: if reveal() { IconName::EyeOff } else { IconName::Eye }, size: 14 }
                    }
                }
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: busy() || value.read().trim().is_empty(),
                    if saved { "Replace" } else { "Save" }
                }
                if saved {
                    button {
                        class: "button button-danger",
                        r#type: "button",
                        disabled: busy(),
                        onclick: remove,
                        "Remove"
                    }
                }
            }

            div { id: "{slug}-credential-help", aria_live: "polite",
                if let Some(message) = error() {
                    p { class: "field-error", role: "alert", "{message}" }
                } else if let Some(message) = notice() {
                    p { class: "field-success", "{message}" }
                }
                p { class: "field-hint",
                    a { href: docs, target: "_blank", rel: "noopener noreferrer", "Where to find {service.label()} API token" }
                }
            }
        }
    }
}
