use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::system::{cache_size, clear_cache, get_system, save_system, SystemView, MAX_DOWNLOAD_CONNECTIONS, MAX_JOBS, THUMB_SIZES};
use crate::components::{use_toasts, InfoTip, Slider, ToastKind};
use crate::state::use_workspace;
use crate::util::format_bytes;

/// Settings > System: worker threads, download connections, jobs at once,
/// preview size, and the open workspace's cache.
#[component]
pub fn SystemSettings() -> Element {
    let loaded = use_resource(get_system);
    let view = loaded.read().as_ref().map(|r| r.as_ref().map(|view| *view).map_err(error_message));
    match view {
        Some(Ok(view)) => rsx! {
            SystemForm { view }
            CacheRow {}
        },
        Some(Err(message)) => rsx! { p { class: "form-error", role: "alert", "{message}" } },
        None => rsx! { p { class: "field-hint", "Loading…" } },
    }
}

#[component]
fn SystemForm(view: SystemView) -> Element {
    // What is stored; replaced after a save.
    let mut stored = use_signal(|| view.prefs);
    let mut prefs = use_signal(|| view.prefs);
    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let cores = view.cores;
    let current = prefs();
    let changed = current != stored();

    let save = move |e: FormEvent| async move {
        e.prevent_default();
        error.set(None);
        saving.set(true);
        match save_system(prefs()).await {
            Ok(saved) => {
                stored.set(saved.prefs);
                prefs.set(saved.prefs);
            }
            Err(e) => error.set(Some(error_message(&e))),
        }
        saving.set(false);
    };

    rsx! {
        form { class: "settings-card form", novalidate: true, onsubmit: save,
            div { class: "field",
                label { class: "field-label", r#for: "system-threads",
                    "Worker threads "
                    InfoTip {
                        label: "About worker threads",
                        text: "Used for copying and extracting files, checking images in Health Check and cutting out crops. Fewer threads leave the computer more responsive while a job runs; jobs take longer. 0 is automatic: all {cores} CPU cores of this computer.",
                    }
                }
                Slider {
                    value: current.threads,
                    label: "Worker threads",
                    max: cores,
                    text: (current.threads == 0).then(|| format!("Automatic ({cores})")),
                    onchange: move |n| prefs.write().threads = n,
                }
            }

            div { class: "field",
                span { class: "field-label",
                    "Download connections "
                    InfoTip {
                        label: "About download connections",
                        text: "Parts of one large file fetched at the same time, when the server allows it.",
                    }
                }
                Slider {
                    value: current.download_connections,
                    label: "Download connections",
                    min: 1,
                    max: MAX_DOWNLOAD_CONNECTIONS,
                    onchange: move |n| prefs.write().download_connections = n,
                }
            }

            div { class: "field",
                span { class: "field-label",
                    "Jobs at once "
                    InfoTip {
                        label: "About jobs at once",
                        text: "With more than one, a second tool can start while another works, e.g. a download during a Health Check. Each tool still runs one job at a time, and a job waits while another one uses the same dataset. Every job uses the worker threads above.",
                    }
                }
                Slider { value: current.max_jobs, label: "Jobs at once", min: 1, max: MAX_JOBS, onchange: move |n| prefs.write().max_jobs = n }
            }

            div { class: "field",
                label { class: "field-label", r#for: "system-thumbs",
                    "Preview size "
                    InfoTip {
                        label: "About preview size",
                        text: "Longest side of the gallery's image previews. Larger ones are sharper, slower to make and take more disk space.",
                    }
                }
                select {
                    id: "system-thumbs",
                    class: "input select",
                    value: "{current.thumb_size}",
                    onchange: move |e| {
                        if let Ok(n) = e.value().parse() {
                            prefs.write().thumb_size = n;
                        }
                    },
                    for size in THUMB_SIZES {
                        option { key: "{size}", value: "{size}", selected: current.thumb_size == size, "{size} px" }
                    }
                }
            }

            if let Some(message) = error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: saving() || !changed,
                    aria_busy: if saving() { "true" },
                    if saving() { "Saving…" } else { "Save" }
                }
                span { class: "form-actions-note", role: "status",
                    if changed { "Unsaved changes" } else { "Saved" }
                }
            }
        }
    }
}

/// Size of the open workspace's cached previews and statistics, and a button to delete them.
#[component]
fn CacheRow() -> Element {
    let workspace = use_workspace();
    let toasts = use_toasts();
    let mut clearing = use_signal(|| false);
    let mut size = use_resource(move || async move {
        let _ = workspace();
        cache_size().await
    });

    let clear = move |_| async move {
        clearing.set(true);
        match clear_cache().await {
            Ok(freed) => toasts.show(ToastKind::Success, format!("Cache cleared: {} freed.", format_bytes(freed))),
            Err(e) => toasts.show(ToastKind::Error, error_message(&e)),
        }
        clearing.set(false);
        size.restart();
    };

    let bytes = match &*size.read() {
        Some(Ok(bytes)) => *bytes,
        _ => None,
    };

    rsx! {
        div { class: "settings-card",
            div { class: "field",
                span { class: "field-label", "Cache" }
                match (workspace(), bytes) {
                    (Some(info), Some(bytes)) => rsx! {
                        div { class: "option-row",
                            button {
                                class: "button button-secondary",
                                r#type: "button",
                                disabled: clearing() || bytes == 0,
                                onclick: clear,
                                if clearing() { "Clearing…" } else { "Clear cache" }
                            }
                            span { class: "field-hint",
                                "{format_bytes(bytes)} of image previews and statistics in "
                                strong { "{info.folder}" }
                                ". They are made again when needed; Health Check reports are kept."
                            }
                        }
                    },
                    (Some(_), None) => rsx! { p { class: "field-hint", "Measuring…" } },
                    (None, _) => rsx! { p { class: "field-hint", "No workspace loaded. The cache belongs to a workspace." } },
                }
            }
        }
    }
}
