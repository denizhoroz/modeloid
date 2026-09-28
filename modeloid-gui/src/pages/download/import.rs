//! Import tab: bring a dataset folder or zip from this computer into the workspace.

use dioxus::prelude::*;

use crate::api::datasets::validate_dataset_name;
use crate::api::error_message;
use crate::api::import::{name_from_path, start_import, ImportKind};
use crate::api::jobs::JobKind;
use crate::api::dialogs::{pick_file, pick_folder, DialogPurpose};
use crate::components::{set_if_newer, use_job, Icon, IconName, JobProgress};

#[derive(Debug, Clone, Copy, PartialEq)]
enum SourceKind {
    Folder,
    Zip,
}

#[component]
pub fn ImportForm(datasets_path: String) -> Element {
    let mut source_kind = use_signal(|| SourceKind::Folder);
    let mut move_folder = use_signal(|| false);
    // Picked path per kind, so switching back keeps the earlier choice.
    let mut folder = use_signal(|| None::<String>);
    let mut zip = use_signal(|| None::<String>);
    // Typed name; `None` until edited, so the path's name is suggested.
    let mut name = use_signal(|| None::<String>);

    let mut picking = use_signal(|| false);
    let mut submitted = use_signal(|| false);
    let mut starting = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);

    let (job, poll_error) = use_job(JobKind::Import);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);

    let path = match source_kind() {
        SourceKind::Folder => folder(),
        SourceKind::Zip => zip(),
    };
    let suggested = path.as_deref().map(name_from_path).unwrap_or_default();
    let name_value = name().unwrap_or_else(|| suggested.clone());
    let path_error = (submitted() && path.is_none()).then(|| match source_kind() {
        SourceKind::Folder => "Choose a dataset folder.",
        SourceKind::Zip => "Choose a zip file.",
    });
    let name_error = if submitted() || name().is_some() { validate_dataset_name(&name_value).err() } else { None };

    let choose = move |_| async move {
        picking.set(true);
        form_error.set(None);
        let picked = match source_kind() {
            SourceKind::Folder => pick_folder("Choose a dataset folder".into(), DialogPurpose::DatasetFolder).await,
            SourceKind::Zip => pick_file("Choose a dataset zip".into(), DialogPurpose::DatasetZip, "Zip archive".into(), vec!["zip".into()]).await,
        };
        match picked {
            Ok(Some(path)) => match source_kind() {
                SourceKind::Folder => folder.set(Some(path)),
                SourceKind::Zip => zip.set(Some(path)),
            },
            Ok(None) => {} // dialog cancelled
            Err(e) => form_error.set(Some(error_message(&e))),
        }
        picking.set(false);
    };

    let submit = move |e: FormEvent| async move {
        e.prevent_default();
        submitted.set(true);
        form_error.set(None);
        let path = match source_kind() {
            SourceKind::Folder => folder(),
            SourceKind::Zip => zip(),
        };
        let Some(path) = path else { return };
        let name_value = name().unwrap_or_else(|| name_from_path(&path));
        if validate_dataset_name(&name_value).is_err() {
            return;
        }
        let kind = match (source_kind(), move_folder()) {
            (SourceKind::Zip, _) => ImportKind::Zip,
            (SourceKind::Folder, true) => ImportKind::Move,
            (SourceKind::Folder, false) => ImportKind::Copy,
        };

        starting.set(true);
        match start_import(path, kind, name_value).await {
            Ok(started) => {
                set_if_newer(job, started);
                submitted.set(false);
                name.set(None);
                match source_kind() {
                    SourceKind::Folder => folder.set(None),
                    SourceKind::Zip => zip.set(None),
                }
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
                div { class: "section-card-header",
                    h2 { id: "import-source-title", class: "section-card-title", "Source" }
                    div { class: "segmented", role: "radiogroup", aria_label: "Import from",
                        for (value, label) in [(SourceKind::Folder, "Folder"), (SourceKind::Zip, "Zip file")] {
                            button {
                                key: "{label}",
                                class: if source_kind() == value { "segmented-option is-selected" } else { "segmented-option" },
                                r#type: "button",
                                role: "radio",
                                aria_checked: if source_kind() == value { "true" } else { "false" },
                                onclick: move |_| source_kind.set(value),
                                "{label}"
                            }
                        }
                    }
                }

                div { class: "folder-pick-row",
                    button {
                        class: "button button-secondary",
                        r#type: "button",
                        disabled: picking() || busy,
                        aria_busy: if picking() { "true" },
                        onclick: choose,
                        Icon { name: IconName::FolderOpen, size: 14 }
                        span {
                            if picking() {
                                "Opening…"
                            } else if path.is_some() {
                                "Change"
                            } else if source_kind() == SourceKind::Folder {
                                "Choose folder"
                            } else {
                                "Choose zip"
                            }
                        }
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
                    if source_kind() == SourceKind::Folder {
                        "A dataset folder in any layout (YOLO, COCO, VOC...). It's scanned and gets a dataset.toml."
                    } else {
                        "A dataset zip. If everything is inside one top folder, that folder is unwrapped."
                    }
                }

                if source_kind() == SourceKind::Folder {
                    div { class: "choice-list",
                        label { class: if !move_folder() { "choice is-selected" } else { "choice" },
                            input {
                                r#type: "radio",
                                name: "import-mode",
                                checked: !move_folder(),
                                onchange: move |_| move_folder.set(false),
                            }
                            span { class: "choice-body",
                                span { class: "choice-label", "Copy" }
                                span { class: "choice-detail", "The original folder stays where it is." }
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
                                span { class: "choice-label", "Move" }
                                span { class: "choice-detail", "Faster on the same drive. The original folder is gone afterwards." }
                            }
                        }
                    }
                }
            }

            section { class: "section-card", aria_labelledby: "import-name-title",
                h2 { id: "import-name-title", class: "section-card-title", "Dataset" }
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
