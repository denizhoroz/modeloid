//! Gallery tab: filtered, paged image grid with label overlays and a detail view.

use dioxus::prelude::*;
use modeloid_types::stats::{ExploreStats, GalleryObject, SPLITS};
use modeloid_types::Split;

use crate::api::error_message;
use crate::api::explore::{gallery_page, image_url, thumb_url, GalleryFilter, GalleryImage};
use crate::util::{format_count, plural};

const PAGE: usize = 60;

#[component]
pub fn Gallery(id: u32, classes: Vec<String>, stats: ExploreStats, filter: Signal<GalleryFilter>) -> Element {
    let mut filter = filter;
    let mut offset = use_signal(|| 0usize);
    let mut show_labels = use_signal(|| true);
    // Index into the current page of the image open in the detail view.
    let mut open = use_signal(|| None::<usize>);

    // A new filter starts from the first page.
    use_effect(move || {
        let _ = filter.read();
        offset.set(0);
        open.set(None);
    });

    let page = use_resource(move || async move { gallery_page(id, filter(), offset(), PAGE).await });

    // Class choices: classes with labels, most frequent first.
    let mut class_options: Vec<(usize, String, u32)> = stats
        .classes
        .iter()
        .enumerate()
        .filter(|(_, c)| c.images > 0)
        .map(|(i, c)| (i, c.name.clone(), c.images))
        .collect();
    class_options.sort_by_key(|(_, _, n)| std::cmp::Reverse(*n));
    let split_options: Vec<Option<Split>> = SPLITS.iter().enumerate().filter(|(i, _)| stats.images[*i] > 0).map(|(_, s)| *s).collect();

    let current = filter();
    let class_value = current.class.map(|c| c.to_string()).unwrap_or_default();
    let split_value = match current.split {
        None => "all".to_string(),
        Some(s) => split_key(s).to_string(),
    };

    rsx! {
        div { class: "gallery-filters", role: "group", aria_label: "Filter images",
            div { class: "field field-inline",
                label { class: "field-label", r#for: "gallery-class", "Class" }
                select {
                    id: "gallery-class",
                    class: "input select",
                    value: "{class_value}",
                    onchange: move |e| {
                        let class = e.value().parse().ok();
                        filter.with_mut(|f| f.class = class);
                    },
                    option { value: "", selected: current.class.is_none(), "All classes" }
                    for (i, name, images) in class_options {
                        option { key: "{i}", value: "{i}", selected: current.class == Some(i), "{name} ({format_count(images)})" }
                    }
                }
            }
            div { class: "field field-inline",
                label { class: "field-label", r#for: "gallery-split", "Split" }
                select {
                    id: "gallery-split",
                    class: "input select",
                    value: "{split_value}",
                    onchange: move |e| {
                        let split = split_from_key(&e.value());
                        filter.with_mut(|f| f.split = split);
                    },
                    option { value: "all", selected: current.split.is_none(), "All splits" }
                    for split in split_options {
                        option { key: "{split_key(split)}", value: "{split_key(split)}", selected: current.split == Some(split), "{Split::label(split)}" }
                    }
                }
            }
            label { class: "check",
                input {
                    r#type: "checkbox",
                    checked: current.unlabeled,
                    onchange: move |e| filter.with_mut(|f| f.unlabeled = e.checked()),
                }
                span { "Only images without labels" }
            }
            label { class: "check",
                input { r#type: "checkbox", checked: show_labels(), onchange: move |e| show_labels.set(e.checked()) }
                span { "Show labels" }
            }
            if current != GalleryFilter::default() {
                button { class: "button button-ghost button-sm", r#type: "button", onclick: move |_| filter.set(GalleryFilter::default()), "Clear filters" }
            }
        }

        match &*page.read() {
            None => rsx! { p { class: "field-hint", aria_live: "polite", "Loading images…" } },
            Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
            Some(Ok(page)) if page.images.is_empty() => rsx! {
                p { class: "panel-empty", "No images match these filters." }
            },
            Some(Ok(page)) => {
                let (first, last) = (page.offset + 1, page.offset + page.images.len());
                let (total, has_prev, has_next) = (page.total, page.offset > 0, last < page.total);
                let images = page.images.clone();
                rsx! {
                    ul { class: "gallery-grid",
                        for (i, image) in images.iter().enumerate() {
                            li { key: "{image.index}",
                                button {
                                    class: "gallery-tile",
                                    r#type: "button",
                                    title: "{image.name}",
                                    onclick: move |_| open.set(Some(i)),
                                    div { class: "gallery-thumb", style: "aspect-ratio: {image.width.max(1)} / {image.height.max(1)}",
                                        img { src: thumb_url(id, image.index), alt: "{image.name}", loading: "lazy", decoding: "async" }
                                        if show_labels() {
                                            Overlay { objects: image.objects.clone(), colors: page.colors.clone(), highlight: current.class }
                                        }
                                    }
                                    span { class: "gallery-caption",
                                        span { class: "gallery-name", "{image.name}" }
                                        span { class: "gallery-meta", "{Split::label(image.split)} · {plural(image.objects.len(), \"object\")}" }
                                    }
                                }
                            }
                        }
                    }
                    nav { class: "pager", aria_label: "Gallery pages",
                        button { class: "button button-secondary button-sm", r#type: "button", disabled: !has_prev,
                            onclick: move |_| offset.set(offset().saturating_sub(PAGE)), "Previous" }
                        span { class: "pager-status", aria_live: "polite", "{format_count(first as u32)}–{format_count(last as u32)} of {format_count(total as u32)}" }
                        button { class: "button button-secondary button-sm", r#type: "button", disabled: !has_next,
                            onclick: move |_| offset.set(offset() + PAGE), "Next" }
                    }
                    if let Some(i) = open() {
                        if let Some(image) = images.get(i).cloned() {
                            ImageDetail {
                                id,
                                image,
                                classes: classes,
                                colors: page.colors.clone(),
                                position: format!("{} of {}", format_count((page.offset + i + 1) as u32), format_count(total as u32)),
                                on_close: move |_| open.set(None),
                                on_step: move |step: isize| {
                                    let next = i as isize + step;
                                    if next >= 0 && (next as usize) < images.len() {
                                        open.set(Some(next as usize));
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}

fn split_key(split: Option<Split>) -> &'static str {
    match split {
        Some(Split::Train) => "train",
        Some(Split::Val) => "val",
        Some(Split::Test) => "test",
        None => "none",
    }
}

/// `None` = no split filter; `Some(None)` = images without a split.
fn split_from_key(key: &str) -> Option<Option<Split>> {
    match key {
        "train" => Some(Some(Split::Train)),
        "val" => Some(Some(Split::Val)),
        "test" => Some(Some(Split::Test)),
        "none" => Some(None),
        _ => None,
    }
}

/// Boxes and outlines over a thumbnail, in the image's 0..1 coordinates.
/// With a class filter, other classes' labels are dimmed.
#[component]
fn Overlay(objects: Vec<GalleryObject>, colors: Vec<String>, highlight: Option<usize>) -> Element {
    rsx! {
        svg { class: "gallery-overlay", view_box: "0 0 1 1", preserve_aspect_ratio: "none", "aria-hidden": "true",
            for (i, o) in objects.iter().enumerate() {
                {
                    let dim = highlight.is_some_and(|h| h != o.class);
                    let class = if dim { "overlay-shape is-dim" } else { "overlay-shape" };
                    // Each class in its own colour (`--shape-color`); the theme's when unknown.
                    let style = colors.get(o.class).map(|c| format!("--shape-color: {c}")).unwrap_or_default();
                    match &o.polygon {
                        Some(points) => {
                            let points: String = points.iter().fold(String::new(), |mut all, [x, y]| {
                                use std::fmt::Write;
                                let _ = write!(all, "{x},{y} ");
                                all
                            });
                            rsx! { polygon { key: "{i}", class, style, points } }
                        }
                        None => rsx! {
                            rect { key: "{i}", class, style, x: "{o.bbox[0]}", y: "{o.bbox[1]}", width: "{o.bbox[2]}", height: "{o.bbox[3]}" }
                        },
                    }
                }
            }
        }
    }
}

/// One image at full size with named boxes. Esc closes, ←/→ step through the page.
#[component]
fn ImageDetail(
    id: u32,
    image: GalleryImage,
    classes: Vec<String>,
    colors: Vec<String>,
    position: String,
    on_close: EventHandler<()>,
    on_step: EventHandler<isize>,
) -> Element {
    let name_of = |c: usize| classes.get(c).cloned().unwrap_or_else(|| format!("class {c}"));
    let mut counts: Vec<(String, usize)> = Vec::new();
    for o in &image.objects {
        let name = name_of(o.class);
        match counts.iter_mut().find(|(n, _)| *n == name) {
            Some((_, n)) => *n += 1,
            None => counts.push((name, 1)),
        }
    }

    rsx! {
        div { class: "scrim scrim-modal", onclick: move |_| on_close.call(()) }
        div {
            class: "image-detail",
            role: "dialog",
            aria_modal: "true",
            aria_label: "{image.name}",
            tabindex: "-1",
            onmounted: move |e| async move {
                let _ = e.set_focus(true).await;
            },
            onkeydown: move |e| match e.key() {
                Key::Escape => on_close.call(()),
                Key::ArrowLeft => on_step.call(-1),
                Key::ArrowRight => on_step.call(1),
                _ => {}
            },
            div { class: "image-detail-head",
                div { class: "image-detail-title",
                    h2 { class: "section-card-title", title: "{image.name}", "{image.name}" }
                    span { class: "section-card-meta", "{position} · {Split::label(image.split)} · {image.width}×{image.height}" }
                }
                button { class: "button button-secondary button-sm", r#type: "button", aria_label: "Previous image", onclick: move |_| on_step.call(-1), "←" }
                button { class: "button button-secondary button-sm", r#type: "button", aria_label: "Next image", onclick: move |_| on_step.call(1), "→" }
                button { class: "button button-sm", r#type: "button", onclick: move |_| on_close.call(()), "Close" }
            }
            div { class: "image-detail-body",
                // As large as the body allows at the image's aspect ratio (container units).
                div {
                    class: "image-detail-frame",
                    style: "width: min(100cqw, calc(100cqh * {image.width.max(1)} / {image.height.max(1)})); aspect-ratio: {image.width.max(1)} / {image.height.max(1)}",
                    img { src: image_url(id, image.index), alt: "{image.name}" }
                    Overlay { objects: image.objects.clone(), colors: colors.clone(), highlight: None }
                    for (i, o) in image.objects.iter().enumerate() {
                        span {
                            key: "{i}",
                            class: if colors.get(o.class).is_some() { "box-tag is-class-colored" } else { "box-tag" },
                            style: "left: {o.bbox[0] * 100.0:.2}%; top: {o.bbox[1] * 100.0:.2}%; --shape-color: {colors.get(o.class).map_or(\"\", String::as_str)}",
                            "{name_of(o.class)}"
                        }
                    }
                }
            }
            p { class: "image-detail-objects",
                if counts.is_empty() {
                    "No labels."
                } else {
                    {counts.iter().map(|(name, n)| format!("{name} × {n}")).collect::<Vec<_>>().join(" · ")}
                }
            }
        }
    }
}
