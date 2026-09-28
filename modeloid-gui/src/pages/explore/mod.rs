//! Dataset Manager > Explore: statistics and a gallery for the dataset
//! selected in the datasets panel.

mod gallery;
mod statistics;

use dioxus::prelude::*;

use crate::api::datasets::list_datasets;
use crate::api::explore::{dataset_stats, GalleryFilter};
use crate::api::error_message;
use crate::components::ChooseDataset;
use crate::routes::Route;
use crate::state::{use_datasets_version, use_selected_dataset, use_workspace, use_workspace_checked};

use gallery::Gallery;
use statistics::Statistics;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Tab {
    Statistics,
    Gallery,
}

#[component]
pub fn Explore() -> Element {
    let workspace = use_workspace();
    let checked = use_workspace_checked();
    let selected = use_selected_dataset();

    rsx! {
        document::Title { "Explore · Modeloid" }
        div { class: "page page-wide",
            p { class: "page-eyebrow", "Dataset Manager" }
            h1 { "Explore" }
            p { class: "page-lead", "Class balance, object sizes and positions, and the images themselves." }

            match (workspace(), selected()) {
                (Some(_), Some(id)) => rsx! { ExploreDataset { key: "{id}", id } },
                (Some(_), None) => rsx! { ChooseDataset {} },
                (None, _) if !checked() => rsx! {},
                (None, _) => rsx! {
                    div { class: "notice",
                        p { "Explore looks at a workspace's datasets. Create one, or load one from the sidebar." }
                        Link { class: "button", to: Route::CreateWorkspace {}, "Create workspace" }
                    }
                },
            }
        }
    }
}

#[component]
fn ExploreDataset(id: u32) -> Element {
    let mut selected = use_selected_dataset();
    let mut tab = use_signal(|| Tab::Statistics);
    let filter = use_signal(GalleryFilter::default);
    let mut recompute = use_signal(|| 0u32);
    let recomputed = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(0u32)));
    let datasets_version = use_datasets_version();

    // Re-fetched when datasets change (the core's cache notices an edited dataset);
    // "Recompute" forces a fresh read once.
    let stats = use_resource(move || {
        let recomputed = recomputed.clone();
        async move {
            let _ = datasets_version();
            let asked = recompute();
            let refresh = asked != recomputed.replace(asked);
            dataset_stats(id, refresh).await
        }
    });
    let datasets = use_resource(move || async move {
        let _ = datasets_version();
        list_datasets().await
    });
    let row = datasets.read().as_ref().and_then(|r| r.as_ref().ok()).and_then(|rows| rows.iter().find(|r| r.id == id).cloned());

    // Clicking a class bar opens the gallery filtered to that class.
    let mut filter_signal = filter;
    let show_class = move |class: usize| {
        filter_signal.set(GalleryFilter { class: Some(class), ..Default::default() });
        tab.set(Tab::Gallery);
    };

    rsx! {
        div { class: "explore-head",
            div { class: "explore-title",
                h2 { class: "explore-name", {row.as_ref().map(|r| r.name.clone()).unwrap_or_else(|| format!("Dataset #{id}"))} }
                if let Some(r) = &row {
                    span { class: "section-card-meta", "#{r.id} · {r.task} · {r.format}" }
                }
            }
            div { class: "explore-actions",
                button {
                    class: "button button-secondary button-sm",
                    r#type: "button",
                    title: "Read the label files again",
                    disabled: stats.read().is_none(),
                    onclick: move |_| *recompute.write() += 1,
                    "Recompute"
                }
                button {
                    class: "button button-ghost button-sm",
                    r#type: "button",
                    onclick: move |_| selected.set(None),
                    "Change dataset"
                }
            }
        }

        div { class: "segmented segmented-tabs", role: "tablist", aria_label: "Explore view",
            for (value, label, slug) in [(Tab::Statistics, "Statistics", "statistics"), (Tab::Gallery, "Gallery", "gallery")] {
                button {
                    key: "{slug}",
                    id: "tab-{slug}",
                    class: if tab() == value { "segmented-option is-selected" } else { "segmented-option" },
                    r#type: "button",
                    role: "tab",
                    aria_selected: if tab() == value { "true" } else { "false" },
                    aria_controls: "panel-{slug}",
                    onclick: move |_| tab.set(value),
                    "{label}"
                }
            }
        }

        match &*stats.read() {
            None => rsx! { p { class: "field-hint", aria_live: "polite", "Reading labels… large datasets take a moment the first time." } },
            Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
            Some(Ok(stats)) => rsx! {
                div { id: "panel-statistics", role: "tabpanel", aria_labelledby: "tab-statistics", hidden: tab() != Tab::Statistics,
                    Statistics { stats: stats.clone(), on_class: show_class }
                }
                div { id: "panel-gallery", role: "tabpanel", aria_labelledby: "tab-gallery", hidden: tab() != Tab::Gallery,
                    Gallery { id, classes: stats.classes.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), stats: stats.clone(), filter }
                }
            },
        }
    }
}
