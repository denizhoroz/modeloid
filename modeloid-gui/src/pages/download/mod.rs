//! Dataset Manager > Download.

mod formats;
mod links;
mod progress;

use dioxus::prelude::*;

use crate::api::download::start_download;
use crate::api::error_message;
use crate::api::settings::{roboflow_key_status, KeyStatus};
use crate::components::{FolderChoice, FolderSelect};
use crate::routes::Route;
use crate::state::{use_workspace, use_workspace_checked};
use crate::util::plural;

use formats::{FormatSection, DEFAULT_FORMAT, OTHER};
use links::{collect_links, LinkMode, LinksSection};
use progress::{set_if_newer, use_download_job, ProgressPanel};

#[component]
pub fn Download() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();

    rsx! {
        document::Title { "Download · Modeloid" }
        div { class: "page",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Download" }
            p { class: "page-lead", "Download datasets from Roboflow into your workspace." }

            match workspace() {
                Some(info) => rsx! { DownloadForm { datasets_path: info.datasets_path } },
                None if !checked() => rsx! {}, // still asking the server
                None => rsx! {
                    div { class: "notice",
                        p { "Downloads go into a workspace. Create one, or load one from the sidebar." }
                        Link { class: "button", to: Route::CreateWorkspace {}, "Create workspace" }
                    }
                },
            }
        }
    }
}

#[component]
fn DownloadForm(datasets_path: String) -> Element {
    let mode = use_signal(|| LinkMode::Single);
    let single = use_signal(String::new);
    let batch = use_signal(String::new);
    let format = use_signal(|| DEFAULT_FORMAT.to_string());
    let custom_format = use_signal(String::new);
    let target = use_signal(FolderChoice::default);

    let mut submitted = use_signal(|| false);
    let mut starting = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);

    let (job, poll_error) = use_download_job();
    let key_status = use_resource(roboflow_key_status);

    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    let format_value = move || if format() == OTHER { custom_format().trim().to_lowercase() } else { format() };
    let format_error = (submitted() && format() == OTHER && format_value().is_empty())
        .then(|| "Enter a format name.".to_string());
    let target_error = if submitted() { target.read().resolve().err() } else { None };
    let link_count = collect_links(mode(), &single.read(), &batch.read()).map_or(0, |l| l.len());

    let no_key = matches!(
        &*key_status.read(),
        Some(Ok(KeyStatus { saved_hint: None, from_env: false, .. }))
    );

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        form_error.set(None);

        let Ok(links) = collect_links(mode(), &single.read(), &batch.read()) else { return };
        let format = format_value();
        if format.is_empty() {
            return;
        }
        let Ok(target) = target.read().resolve() else { return };

        starting.set(true);
        match start_download(links, format, target).await {
            Ok(started) => {
                set_if_newer(job, started);
                submitted.set(false);
            }
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        starting.set(false);
    };

    rsx! {
        if no_key {
            div { class: "notice notice-warning", role: "status",
                p { "No Roboflow API key is set, so downloads will fail." }
                Link { class: "button button-secondary", to: Route::Settings {}, "Add key in Settings" }
            }
        }

        form { class: "section-stack", novalidate: true, onsubmit: submit,
            LinksSection { mode, single, batch, show_errors: submitted() }
            FormatSection { selected: format, custom: custom_format, error: format_error }
            FolderSelect {
                title: "Target folder",
                default_label: "Workspace datasets folder",
                default_path: datasets_path,
                choice: target,
                dialog_title: "Choose where to save the datasets",
                error: target_error,
            }

            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }

            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: running || starting(),
                    aria_busy: if starting() { "true" },
                    if running {
                        "Downloading…"
                    } else if link_count > 1 {
                        "Download {plural(link_count, \"dataset\")}"
                    } else {
                        "Download"
                    }
                }
                if running {
                    span { class: "form-actions-note", "Wait for the current download to finish." }
                }
            }
        }

        if let Some(message) = poll_error() {
            p { class: "form-error", role: "alert", "Lost track of download progress: {message}. Reload the page." }
        }
        if let Some(current) = job() {
            ProgressPanel { job: current }
        }
    }
}
