//! Dataset Manager > Datasets: make datasets (create an empty one, edit its
//! details and add images), bring them into the workspace (download from
//! online sources, import a folder / zip from this computer) and delete them
//! from it.

mod create;
mod edit;
mod formats;
mod import;
mod links;

use dioxus::prelude::*;

use crate::api::download::{download_forking, is_hugging_face, link_service, parse_links, start_download};
use crate::api::jobs::{ItemAction, JobKind};
use crate::api::error_message;
use crate::api::settings::{credential_status, KeyStatus, ServiceId};
use crate::components::{use_toasts, NoWorkspace, set_if_newer, use_job, JobProgress};
use crate::routes::Route;
use crate::state::{use_workspace, use_workspace_checked};
use crate::util::plural;

use create::CreateForm;
use edit::EditForm;
use formats::{FormatSection, DEFAULT_FORMAT, OTHER};
use import::ImportForm;
use links::{collect_links, LinkMode, LinksSection};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Create,
    Edit,
    Download,
    Import,
}

#[component]
pub fn Download() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let mut tab = use_signal(|| Tab::Create);
    // Coming back from the labelling interface: open on Edit.
    use_hook(move || {
        if *crate::pages::label::RETURN_TO_EDIT.peek() {
            *crate::pages::label::RETURN_TO_EDIT.write() = false;
            tab.set(Tab::Edit);
        }
    });

    rsx! {
        document::Title { "Datasets · Modeloid" }
        div { class: "page",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Datasets" }
            p { class: "page-lead",
                "Create an empty dataset and add your own images, edit a dataset's details, download datasets from Roboflow, Kaggle, Hugging Face or a zip link, or import one from this computer. To delete datasets, select them in the Datasets table and choose Remove."
            }

            match workspace() {
                Some(info) => rsx! {
                    div { class: "segmented segmented-tabs", role: "tablist", aria_label: "Dataset actions",
                        for (value, label, id) in [
                            (Tab::Create, "Create", "create"),
                            (Tab::Edit, "Edit", "edit"),
                            (Tab::Download, "Download", "download"),
                            (Tab::Import, "Import", "import"),
                        ] {
                            button {
                                key: "{id}",
                                id: "tab-{id}",
                                class: if tab() == value { "segmented-option is-selected" } else { "segmented-option" },
                                r#type: "button",
                                role: "tab",
                                aria_selected: if tab() == value { "true" } else { "false" },
                                aria_controls: "panel-{id}",
                                onclick: move |_| tab.set(value),
                                "{label}"
                            }
                        }
                    }
                    // All stay mounted so typed input and progress survive switching tabs.
                    div { id: "panel-create", role: "tabpanel", aria_labelledby: "tab-create", hidden: tab() != Tab::Create,
                        CreateForm {
                            datasets_path: info.datasets_path.clone(),
                            default_format: info.default_format.clone(),
                            on_add_images: move |_| tab.set(Tab::Edit),
                        }
                    }
                    div { id: "panel-edit", role: "tabpanel", aria_labelledby: "tab-edit", hidden: tab() != Tab::Edit,
                        EditForm { datasets_path: info.datasets_path.clone() }
                    }
                    div { id: "panel-download", role: "tabpanel", aria_labelledby: "tab-download", hidden: tab() != Tab::Download,
                        DownloadForm { datasets_path: info.datasets_path.clone() }
                    }
                    div { id: "panel-import", role: "tabpanel", aria_labelledby: "tab-import", hidden: tab() != Tab::Import,
                        ImportForm { datasets_path: info.datasets_path }
                    }
                },
                None if !checked() => rsx! {}, // still asking the server
                None => rsx! { NoWorkspace { text: "Datasets go into a workspace." } },
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

    let mut submitted = use_signal(|| false);
    let mut starting = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);

    let (job, poll_error) = use_job(JobKind::Download);
    let toasts = use_toasts();
    let roboflow = use_resource(move || credential_status(ServiceId::Roboflow));
    let kaggle = use_resource(move || credential_status(ServiceId::Kaggle));

    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    let format_value = move || if format() == OTHER { custom_format().trim().to_lowercase() } else { format() };
    let format_error = (submitted() && format() == OTHER && format_value().is_empty())
        .then(|| "Enter a format name.".to_string());
    let link_count = collect_links(mode(), &single.read(), &batch.read()).map_or(0, |l| l.len());

    // Services the typed links need that have no credentials anywhere.
    let typed = match mode() {
        LinkMode::Single => vec![single().trim().to_string()],
        LinkMode::Batch => parse_links(&batch.read()),
    };
    let missing: Vec<ServiceId> = [(ServiceId::Roboflow, roboflow), (ServiceId::Kaggle, kaggle)]
        .into_iter()
        .filter(|(service, _)| typed.iter().any(|l| link_service(l) == Some(*service)))
        .filter(|(_, status)| {
            matches!(&*status.read(), Some(Ok(KeyStatus { saved_hint: None, fallback: None, .. })))
        })
        .map(|(service, _)| service)
        .collect();

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        form_error.set(None);

        let Ok(links) = collect_links(mode(), &single.read(), &batch.read()) else { return };
        let format = format_value();
        if format.is_empty() {
            return;
        }

        let from_hugging_face = links.iter().any(|l| is_hugging_face(l));
        starting.set(true);
        match start_download(links, format).await {
            Ok(started) => {
                set_if_newer(job, started);
                submitted.set(false);
                if from_hugging_face {
                    toasts.info("Hugging Face downloads through the API can be slow. If this one takes too long, cancel it, download the files from the website and bring them in with Import.");
                }
            }
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        starting.set(false);
    };

    // The button on an item that ended as a warning: run that one link again,
    // taking the offered way. It runs inside the same job, so the list stays.
    let take_action = move |(index, action): (usize, ItemAction)| async move {
        let ItemAction::RoboflowFork = action;
        let format = format_value();
        let link = job.read().as_ref().and_then(|j| j.items.get(index).map(|item| item.label.clone()));
        let Some(link) = link else { return };
        if format.is_empty() || running || starting() {
            return;
        }
        form_error.set(None);
        starting.set(true);
        match download_forking(index, link, format).await {
            Ok(started) => set_if_newer(job, started),
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        starting.set(false);
    };

    rsx! {
        for service in missing {
            div { key: "{service.slug()}", class: "notice notice-warning", role: "status",
                p { "No {service.credential()} set, so {service.label()} downloads will fail." }
                Link { class: "button button-secondary", to: Route::Settings {}, "Add in Settings" }
            }
        }

        form { class: "section-stack", novalidate: true, onsubmit: submit,
            LinksSection { mode, single, batch, show_errors: submitted() }
            FormatSection { selected: format, custom: custom_format, error: format_error }
            p { class: "field-hint", "Datasets are saved in the workspace's datasets folder: "
                code { "{datasets_path}" }
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
            JobProgress { job: current, title: "Downloads", noun: "dataset", verb: "downloaded", on_action: take_action }
        }
    }
}
