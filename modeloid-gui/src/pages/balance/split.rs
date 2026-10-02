//! Split tab: deal the images into train / val / test with a three-segment
//! bar whose two boundaries can be dragged.

use std::rc::Rc;

use dioxus::prelude::*;

use super::{use_preview, ResultCard, RunCard, SeedField};
use crate::api::balance::{BalancePlan, Resplit};
use crate::api::jobs::JobKind;
use crate::components::InfoTip;
use crate::util::format_count;

const NAMES: [&str; 3] = ["Train", "Val", "Test"];

#[component]
pub fn SplitTab(id: u32, name: String) -> Element {
    let mut plan = use_signal(|| BalancePlan::new(id));
    let (preview, error) = use_preview(plan);
    let shares = match plan.read().split {
        Resplit::Ratios { train, val, test } => [train, val, test],
        Resplit::Keep => [100, 0, 0],
    };
    let stratify = plan.read().stratify;

    rsx! {
        if let Some(message) = error() {
            p { class: "form-error", role: "alert", "{message}" }
        }
        match preview() {
            Some(p) => rsx! {
                div { class: "section-stack",
                    section { class: "section-card", aria_labelledby: "split-title",
                        div { class: "section-card-header",
                            h2 { id: "split-title", class: "section-card-title", "Split" }
                            span { class: "section-card-meta", "drag the two boundaries" }
                        }
                        SplitBar {
                            shares,
                            onchange: move |[train, val, test]: [u32; 3]| plan.write().split = Resplit::Ratios { train, val, test },
                        }
                        dl { class: "split-legend",
                            for (slot, label) in NAMES.into_iter().enumerate() {
                                div { key: "{label}", class: "split-legend-item",
                                    dt { "{label}" }
                                    dd { class: "split-legend-share", "{shares[slot]}%" }
                                    dd { class: "split-legend-count", "{format_count(p.images_after[slot])} images" }
                                }
                            }
                        }
                        label { class: "check",
                            input {
                                r#type: "checkbox",
                                checked: stratify,
                                onchange: move |e| plan.write().stratify = e.checked(),
                            }
                            span {
                                "Stratify by class "
                                InfoTip { label: "About stratifying", text: "Each split gets a similar class mix." }
                            }
                        }
                        SeedField { plan, used: p.seed, hint: "The same seed gives the same split again." }
                    }
                    ResultCard { preview: p.clone(), id: "split-result-title" }
                    RunCard { plan: plan(), can_run: p.can_run(), name, suffix: "split", kind: JobKind::Split, action: "Split" }
                }
            },
            None if error().is_some() => rsx! {},
            None => rsx! { p { class: "field-hint", "Reading labels…" } },
        }
    }
}

/// Train | val | test as one bar. The two boundaries are sliders: drag them,
/// or focus one and use the arrow keys (Shift for steps of 5). Shares are
/// whole percentages that add up to 100; train keeps at least 1%.
#[component]
fn SplitBar(shares: [u32; 3], onchange: EventHandler<[u32; 3]>) -> Element {
    let mut bar = use_signal(|| None::<Rc<MountedData>>);
    // While dragging: the boundary, and the bar's left edge and width in px.
    let mut drag = use_signal(|| None::<(usize, f64, f64)>);
    let [train, val, _] = shares;
    // Where the boundaries sit, in percent from the left.
    let bounds = [i64::from(train), i64::from(train + val)];

    let move_to = move |boundary: usize, percent: i64| {
        let [first, second] = bounds;
        let (first, second) = if boundary == 0 { (percent.clamp(1, second), second) } else { (first, percent.clamp(first, 100)) };
        let next = [first as u32, (second - first) as u32, (100 - second) as u32];
        if next != shares {
            onchange.call(next);
        }
    };

    rsx! {
        div { class: "split-bar", onmounted: move |e| bar.set(Some(e.data())),
            for (slot, label) in NAMES.into_iter().enumerate() {
                div {
                    key: "{label}",
                    class: "split-segment split-segment-{slot}",
                    style: "width: {shares[slot]}%",
                    // Too narrow for text: the legend below has the numbers.
                    if shares[slot] >= 8 {
                        span { class: "split-segment-name", "{label}" }
                        span { class: "split-segment-share", "{shares[slot]}%" }
                    }
                }
            }
            for (boundary, label) in ["Train and val boundary", "Val and test boundary"].into_iter().enumerate() {
                div {
                    key: "{boundary}",
                    class: if drag().is_some_and(|d| d.0 == boundary) { "split-handle is-dragging" } else { "split-handle" },
                    style: "left: {bounds[boundary]}%",
                    role: "slider",
                    tabindex: "0",
                    aria_label: "{label}",
                    aria_orientation: "horizontal",
                    aria_valuemin: "0",
                    aria_valuemax: "100",
                    aria_valuenow: "{bounds[boundary]}",
                    aria_valuetext: "Train {shares[0]}%, val {shares[1]}%, test {shares[2]}%",
                    onpointerdown: move |e| {
                        e.prevent_default();
                        spawn(async move {
                            let Some(element) = bar() else { return };
                            if let Ok(rect) = element.get_client_rect().await {
                                drag.set(Some((boundary, rect.origin.x, rect.size.width.max(1.0))));
                            }
                        });
                    },
                    onkeydown: move |e| {
                        let step = if e.modifiers().shift() { 5 } else { 1 };
                        match e.key() {
                            Key::ArrowLeft | Key::ArrowDown => move_to(boundary, bounds[boundary] - step),
                            Key::ArrowRight | Key::ArrowUp => move_to(boundary, bounds[boundary] + step),
                            Key::Home => move_to(boundary, 0),
                            Key::End => move_to(boundary, 100),
                            _ => return,
                        }
                        e.prevent_default();
                    },
                }
            }
        }

        // While dragging, a full-screen layer catches the pointer (see `ResizeHandle`).
        if let Some((boundary, left, width)) = drag() {
            div {
                class: "resize-overlay",
                onpointermove: move |e| move_to(boundary, ((e.client_coordinates().x - left) / width * 100.0).round() as i64),
                onpointerup: move |_| drag.set(None),
                onpointerleave: move |_| drag.set(None),
            }
        }
    }
}
