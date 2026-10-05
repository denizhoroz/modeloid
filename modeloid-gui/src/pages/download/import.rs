//! Import tab: bring a dataset folder or zip from this computer into the workspace.

use dioxus::prelude::*;

use crate::api::datasets::validate_dataset_name;
use crate::components::TitleIcon;
use crate::api::error_message;
use crate::api::import::{name_from_path, start_import, ImportKind};
use crate::api::jobs::JobKind;
use crate::api::dialogs::{pick_file, pick_folder, DialogPurpose};
use crate::components::{set_if_newer, use_job, Icon, IconName, InfoTip, JobProgress};

/// A picked path is a zip by its extension; anything else is a folder.
fn is_zip(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".zip")
}

#[component]
pub fn ImportForm(datasets_path: String) -> Element {
    let mut move_folder = use_signal(|| false);
    // A folder or a zip; which one is told from the path.
    let mut picked = use_signal(|| None::<String>);
    // Typed name; `None` until edited, so the path's name is suggested.
    let mut name = use_signal(|| None::<String>);

    let mut picking = use_signal(|| false);
    let mut submitted = use_signal(|| false);
    let mut starting = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);

    let (job, poll_error) = use_job(JobKind::Import);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);

    let path = picked();
    let zip_picked = path.as_deref().is_some_and(is_zip);
    let suggested = path.as_deref().map(name_from_path).unwrap_or_default();
    let name_value = name().unwrap_or_else(|| suggested.clone());
    let path_error = (submitted() && path.is_none()).then_some("Choose a dataset folder or zip file.");
    let name_error = if submitted() || name().is_some() { validate_dataset_name(&name_value).err() } else { None };

    // The system dialogs pick either a folder or a file, so there's one button each.
    let choose = move |zip_file: bool| async move {
        picking.set(true);
        form_error.set(None);
        let result = if zip_file {
            pick_file("Choose a dataset zip".into(), DialogPurpose::DatasetZip, "Zip archive".into(), vec!["zip".into()]).await
        } else {
            pick_folder("Choose a dataset folder".into(), DialogPurpose::DatasetFolder).await
        };
        match result {
            Ok(Some(path)) => picked.set(Some(path)),
            Ok(None) => {} // dialog cancelled
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        picking.set(false);
    };

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        form_error.set(None);
        let Some(path) = picked() else { return };
        let name_value = name().unwrap_or_else(|| name_from_path(&path));
        if validate_dataset_name(&name_value).is_err() {
            return;
        }
        let kind = match (is_zip(&path), move_folder()) {
            (true, _) => ImportKind::Zip,
            (false, true) => ImportKind::Move,
            (false, false) => ImportKind::Copy,
        };

        starting.set(true);
        match start_import(path, kind, name_value).await {
            Ok(started) => {
                set_if_newer(job, started);
                submitted.set(false);
                name.set(None);
                picked.set(None);
            }
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        starting.set(false);
    };

    let busy = running || starting();
    let sep = if datasets_path.contains('\\') { "\\" } else { "/" };

    rsx! {
        form { class: "section-stack", novalidate: true, onsubmit: submit,
            section { class: "section-card", aria_labelledby: "import-source-title",
                h2 { id: "import-source-title", class: "section-card-title", TitleIcon { name: IconName::Upload } "Source" }

                div { class: "folder-pick-row",
                    button {
                        class: "button button-secondary",
                        r#type: "button",
                        disabled: picking() || busy,
                        aria_busy: if picking() { "true" },
                        onclick: move |_| choose(false),
                        Icon { name: IconName::FolderOpen, size: 14 }
                        span { "Choose folder" }
                    }
                    button {
                        class: "button button-secondary",
                        r#type: "button",
                        disabled: picking() || busy,
                        aria_busy: if picking() { "true" },
                        onclick: move |_| choose(true),
                        Icon { name: IconName::Package, size: 14 }
                        span { "Choose zip" }
                    }
                    match &path {
                        Some(p) => rsx! { code { class: "picked-path", title: "{p}", "{p}" } },
                        None => rsx! { span { class: "choice-detail is-empty", "Nothing chosen" } },
                    }
                }
                if let Some(message) = path_error {
                    p { class: "field-error", role: "alert", "{message}" }
                }
                p { class: "field-hint",
                    "A dataset folder or zip in any layout (YOLO, COCO, VOC...). A zip is unpacked, one top folder unwrapped; either way it is scanned and gets a dataset.toml."
                }

                if path.is_some() && !zip_picked {
                    div { class: "choice-list",
                        label { class: if !move_folder() { "choice is-selected" } else { "choice" },
                            input {
                                r#type: "radio",
                                name: "import-mode",
                                checked: !move_folder(),
                                onchange: move |_| move_folder.set(false),
                            }
                            span { class: "choice-body",
                                span { class: "choice-label",
                                    "Copy "
                                    InfoTip { label: "About copying", text: "The original folder stays where it is." }
                                }
                            }
                        }
                        label { class: if move_folder() { "choice is-selected" } else { "choice" },
                            input {
                                r#type: "radio",
                                name: "import-mode",
                                checked: move_folder(),
                                onchange: move |_| move_folder.set(true),
                            }
                            span { class: "choice-body",
                                span { class: "choice-label",
                                    "Move "
                                    InfoTip { label: "About moving", text: "Faster on the same drive. The original folder is gone afterwards." }
                                }
                            }
                        }
                    }
                }
            }

            section { class: "section-card", aria_labelledby: "import-name-title",
                h2 { id: "import-name-title", class: "section-card-title", TitleIcon { name: IconName::Database } "Dataset" }
                div { class: "field",
                    label { class: "field-label", r#for: "import-name", "Name" }
                    input {
                        id: "import-name",
                        class: "input",
                        r#type: "text",
                        spellcheck: "false",
                        autocomplete: "off",
                        placeholder: "Taken from the folder or zip name",
                        value: "{name_value}",
                        aria_invalid: if name_error.is_some() { "true" },
                        aria_describedby: "import-name-help",
                        oninput: move |e| name.set(Some(e.value())),
                    }
                    match &name_error {
                        Some(message) => rsx! { p { id: "import-name-help", class: "field-error", role: "alert", "{message}" } },
                        None => rsx! {
                            p { id: "import-name-help", class: "field-hint", "Saved as "
                                code { "{datasets_path}{sep}{name_value}" }
                            }
                        },
                    }
                }
            }

            if let Some(message) = form_error() {
                p { class: "form-error", role: "alert", "{message}" }
            }

            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: busy,
                    aria_busy: if starting() { "true" },
                    if running { "Importing…" } else { "Import" }
                }
                if running {
                    span { class: "form-actions-note", "Wait for the current import to finish." }
                }
            }
        }

        if let Some(message) = poll_error() {
            p { class: "form-error", role: "alert", "Lost track of import progress: {message}. Reload the page." }
        }
        if let Some(current) = job() {
            JobProgress { job: current, title: "Import", noun: "dataset", verb: "imported" }
        }
    }
}
