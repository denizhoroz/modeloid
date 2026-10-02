//! Edit tab: the dataset selected last in the datasets panel. Change its
//! details, add images to it and see how far its labelling is.

use dioxus::fullstack::FileStream;
use dioxus::html::{FileData, HasFileData};
use dioxus::prelude::*;

use crate::api::datasets::validate_dataset_name;
use crate::api::edit::{
    dataset_details, is_image_name, label_status, parse_tags, save_dataset_details, start_add_images, start_upload,
    upload_image, DatasetDetails,
};
use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::components::{set_if_newer, use_job, use_toasts, Icon, IconName, JobProgress, PrimaryNote, SelectDatasetHint, StatTile};
use crate::state::{use_datasets_version, use_primary_dataset, use_selected_datasets};
use crate::util::{format_count, plural};

#[component]
pub fn EditForm(datasets_path: String) -> Element {
    let primary = use_primary_dataset();
    let selected = use_selected_datasets();

    rsx! {
        match primary() {
            Some(id) => rsx! {
                EditDataset { key: "{id}", id, datasets_path, selected_count: selected.read().len() }
            },
            None => rsx! { SelectDatasetHint {} },
        }
    }
}

/// Edit for dataset `id`. Keyed by id, so another selection starts fresh.
#[component]
fn EditDataset(id: u32, datasets_path: String, selected_count: usize) -> Element {
    let datasets_version = use_datasets_version();
    let details = use_resource(move || async move {
        let _ = datasets_version();
        dataset_details(id).await
    });

    // The last details read stay shown while they're read again (after a
    // save), so the form isn't unmounted and keeps its "Saved." note.
    let mut shown = use_signal(|| None::<DatasetDetails>);
    use_effect(move || {
        if let Some(Ok(loaded)) = &*details.read() {
            shown.set(Some(loaded.clone()));
        }
    });

    let error = details.read().as_ref().and_then(|r| r.as_ref().err()).map(error_message);
    rsx! {
        PrimaryNote { count: selected_count }
        div { class: "section-stack",
            match (shown(), error) {
                (_, Some(message)) => rsx! { p { class: "form-error", role: "alert", "Couldn't read the dataset: {message}" } },
                (Some(details), None) => rsx! { DetailsSection { id, details, datasets_path } },
                (None, None) => rsx! { p { class: "field-hint", "Loading…" } },
            }
            ImagesSection { id }
            LabellingSection { id }
        }
    }
}

/// Name, description, link, license and tags. Fields keep what was typed
/// (`Some`) until saved; untouched ones follow the stored `details`.
#[component]
fn DetailsSection(id: u32, details: DatasetDetails, datasets_path: String) -> Element {
    let mut name = use_signal(|| None::<String>);
    let mut description = use_signal(|| None::<String>);
    let mut link = use_signal(|| None::<String>);
    let mut license = use_signal(|| None::<String>);
    let mut tags = use_signal(|| None::<String>);

    let mut saving = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut saved = use_signal(|| false);
    let mut datasets_version = use_datasets_version();

    let name_value = name().unwrap_or_else(|| details.name.clone());
    let description_value = description().unwrap_or_else(|| details.description.clone());
    let link_value = link().unwrap_or_else(|| details.link.clone());
    let license_value = license().unwrap_or_else(|| details.license.clone());
    let tags_value = tags().unwrap_or_else(|| details.tags.join(", "));

    let edited = DatasetDetails {
        name: name_value.clone(),
        description: description_value.trim().to_string(),
        link: link_value.trim().to_string(),
        license: license_value.trim().to_string(),
        tags: parse_tags(&tags_value),
    };
    let changed = edited != details;
    let name_error = validate_dataset_name(&name_value).err();
    let sep = if datasets_path.contains('\\') { "\\" } else { "/" };

    let stored = details.clone();
    let save = move |e: FormEvent| {
        let edited = edited.clone();
        let expected = stored.name.clone();
        async move {
            e.prevent_default();
            error.set(None);
            saved.set(false);
            if validate_dataset_name(&edited.name).is_err() {
                return;
            }
            saving.set(true);
            match save_dataset_details(id, expected, edited).await {
                Ok(()) => {
                    for mut field in [name, description, link, license, tags] {
                        field.set(None);
                    }
                    saved.set(true);
                    *datasets_version.write() += 1;
                }
                Err(e) => error.set(Some(error_message(&e))),
            }
            saving.set(false);
        }
    };

    let reset = move |_| {
        for mut field in [name, description, link, license, tags] {
            field.set(None);
        }
        error.set(None);
    };

    rsx! {
        form { class: "section-card", novalidate: true, aria_labelledby: "edit-details-title", onsubmit: save,
            h2 { id: "edit-details-title", class: "section-card-title", "Details" }
            div { class: "field",
                label { class: "field-label", r#for: "edit-name", "Name" }
                input {
                    id: "edit-name",
                    class: "input",
                    r#type: "text",
                    spellcheck: "false",
                    autocomplete: "off",
                    value: "{name_value}",
                    aria_invalid: if name_error.is_some() { "true" },
                    aria_describedby: "edit-name-help",
                    oninput: move |e| name.set(Some(e.value())),
                }
                match &name_error {
                    Some(message) => rsx! { p { id: "edit-name-help", class: "field-error", role: "alert", "{message}" } },
                    None if name_value != details.name => rsx! {
                        p { id: "edit-name-help", class: "field-hint", "The folder is renamed to "
                            code { "{datasets_path}{sep}{name_value}" }
                        }
                    },
                    None => rsx! {},
                }
            }
            div { class: "field",
                label { class: "field-label", r#for: "edit-description",
                    "Description"
                    span { class: "field-optional", " (optional)" }
                }
                textarea {
                    id: "edit-description",
                    class: "input textarea",
                    rows: "3",
                    value: "{description_value}",
                    oninput: move |e| description.set(Some(e.value())),
                }
            }
            div { class: "field",
                label { class: "field-label", r#for: "edit-link",
                    "Link"
                    span { class: "field-optional", " (optional)" }
                }
                input {
                    id: "edit-link",
                    class: "input",
                    r#type: "url",
                    spellcheck: "false",
                    autocomplete: "off",
                    placeholder: "Where the dataset comes from",
                    value: "{link_value}",
                    oninput: move |e| link.set(Some(e.value())),
                }
            }
            div { class: "field",
                label { class: "field-label", r#for: "edit-license",
                    "License"
                    span { class: "field-optional", " (optional)" }
                }
                input {
                    id: "edit-license",
                    class: "input",
                    r#type: "text",
                    autocomplete: "off",
                    placeholder: "e.g. CC BY 4.0",
                    value: "{license_value}",
                    oninput: move |e| license.set(Some(e.value())),
                }
            }
            div { class: "field",
                label { class: "field-label", r#for: "edit-tags",
                    "Tags"
                    span { class: "field-optional", " (optional)" }
                }
                input {
                    id: "edit-tags",
                    class: "input",
                    r#type: "text",
                    autocomplete: "off",
                    placeholder: "e.g. cars, night, drone",
                    aria_describedby: "edit-tags-help",
                    value: "{tags_value}",
                    oninput: move |e| tags.set(Some(e.value())),
                }
                p { id: "edit-tags-help", class: "field-hint", "Separate tags with commas." }
            }

            if let Some(message) = error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "submit",
                    disabled: saving() || !changed || name_error.is_some(),
                    aria_busy: if saving() { "true" },
                    if saving() { "Saving…" } else { "Save" }
                }
                if changed {
                    button { class: "button button-ghost", r#type: "button", disabled: saving(), onclick: reset, "Undo changes" }
                } else if saved() {
                    span { class: "field-success", role: "status", "Saved." }
                }
            }
        }
    }
}

/// Drop images on the zone, or choose them; they are uploaded, then a job
/// moves them into the dataset.
#[component]
fn ImagesSection(id: u32) -> Element {
    let (job, poll_error) = use_job(JobKind::AddImages);
    let mut hover = use_signal(|| false);
    // Images uploaded so far of all being uploaded.
    let mut uploading = use_signal(|| None::<(usize, usize)>);
    let mut error = use_signal(|| None::<String>);
    // Show the job only when it was started here, for this dataset.
    let mut started_here = use_signal(|| false);
    let toasts = use_toasts();

    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    let busy = running || uploading().is_some();

    let add = move |files: Vec<FileData>| {
        spawn(async move {
            if busy {
                return;
            }
            error.set(None);
            let (images, others): (Vec<FileData>, Vec<FileData>) = files.into_iter().partition(|f| is_image_name(&f.name()));
            if images.is_empty() {
                if !others.is_empty() {
                    error.set(Some("None of those files is an image (jpg, png, bmp, webp, tif).".into()));
                }
                return;
            }
            if !others.is_empty() {
                toasts.info(format!("Skipped {} that aren't images.", plural(others.len(), "file")));
            }

            let total = images.len();
            uploading.set(Some((0, total)));
            let result: Result<(), String> = async {
                let token = start_upload().await.map_err(|e| error_message(&e))?;
                for (done, file) in images.into_iter().enumerate() {
                    let file_name = file.name();
                    upload_image(token.clone(), FileStream::from(file))
                        .await
                        .map_err(|e| format!("Couldn't upload {file_name}: {}", error_message(&e)))?;
                    uploading.set(Some((done + 1, total)));
                }
                let started = start_add_images(id, token).await.map_err(|e| error_message(&e))?;
                started_here.set(true);
                set_if_newer(job, started);
                Ok(())
            }
            .await;
            if let Err(message) = result {
                error.set(Some(message));
            }
            uploading.set(None);
        });
    };

    let zone_class = match (hover(), busy) {
        (_, true) => "drop-zone is-busy",
        (true, false) => "drop-zone is-hover",
        (false, false) => "drop-zone",
    };

    let zone_text = match uploading() {
        Some((done, total)) => format!("Uploading {done} of {}…", plural(total, "image")),
        None if running => "Adding images…".to_string(),
        None => "Drop images here, or".to_string(),
    };

    rsx! {
        section { class: "section-card", aria_labelledby: "edit-images-title",
            h2 { id: "edit-images-title", class: "section-card-title", "Add images" }
            div {
                class: zone_class,
                ondragenter: move |e| {
                    e.prevent_default();
                    hover.set(true);
                },
                ondragover: move |e| e.prevent_default(),
                ondragleave: move |_| hover.set(false),
                ondrop: move |e| {
                    e.prevent_default();
                    hover.set(false);
                    add(e.files());
                },
                Icon { name: IconName::FolderOpen, size: 20 }
                p { class: "drop-zone-text", "{zone_text}" }
                label { class: if busy { "button button-secondary is-disabled" } else { "button button-secondary" },
                    input {
                        class: "visually-hidden",
                        r#type: "file",
                        multiple: true,
                        accept: "image/*",
                        disabled: busy,
                        onchange: move |e| add(e.files()),
                    }
                    "Choose images"
                }
            }
            p { class: "field-hint", "Images are copied into the dataset's images folder. Names already taken get a number added." }
            if let Some(message) = error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
        }

        if let Some(message) = poll_error() {
            p { class: "form-error", role: "alert", "Lost track of the progress: {message}. Reload the page." }
        }
        if started_here() {
            if let Some(current) = job() {
                JobProgress { job: current, title: "Add images", noun: "dataset", verb: "updated" }
            }
        }
    }
}

/// How many images still need labels. The labelling tool itself is still to come.
#[component]
fn LabellingSection(id: u32) -> Element {
    let datasets_version = use_datasets_version();
    let status = use_resource(move || async move {
        let _ = datasets_version();
        label_status(id).await
    });
    let toasts = use_toasts();

    let current = status.read().clone();
    rsx! {
        section { class: "section-card", aria_labelledby: "edit-labelling-title",
            h2 { id: "edit-labelling-title", class: "section-card-title", "Labelling" }
            match current {
                Some(Ok(status)) => {
                    let labelled = status.images - status.unlabelled;
                    rsx! {
                        div { class: "stat-tiles",
                            StatTile { label: "Not labelled", value: format_count(status.unlabelled), detail: format!("of {}", plural(status.images as usize, "image")) }
                            StatTile { label: "Labelled", value: format_count(labelled) }
                            StatTile { label: "Labels", value: format_count(status.labels), detail: if status.classes == 1 { "1 class".to_string() } else { format!("{} classes", status.classes) } }
                        }
                    }
                }
                Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "Couldn't read the labels: {error_message(&e)}" } },
                None => rsx! { p { class: "field-hint", "Reading labels…" } },
            }
            div { class: "form-actions",
                button {
                    class: "button",
                    r#type: "button",
                    onclick: move |_| toasts.info("The labelling interface isn't built yet."),
                    "Open labelling"
                }
            }
        }
    }
}
