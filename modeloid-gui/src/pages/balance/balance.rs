//! Balance tab: a class distribution chart to leave classes out and to cut
//! dominant classes down to a threshold, plus a random subset.

use std::rc::Rc;

use dioxus::prelude::*;

use super::{use_preview, ResultCard, RunCard, SeedField};
use crate::api::balance::{BalancePlan, BalancePreview};
use crate::api::jobs::JobKind;
use crate::components::NumberInput;
use crate::util::format_count;

#[component]
pub fn BalanceTab(id: u32, name: String) -> Element {
    let mut plan = use_signal(|| BalancePlan { input: id, ..Default::default() });
    let (preview, error) = use_preview(plan);
    let subset = plan.read().subset_percent;

    rsx! {
        if let Some(message) = error() {
            p { class: "form-error", role: "alert", "{message}" }
        }
        match preview() {
            Some(p) => rsx! {
                div { class: "section-stack",
                    section { class: "section-card", aria_labelledby: "distribution-title",
                        div { class: "section-card-header",
                            h2 { id: "distribution-title", class: "section-card-title", "Class Distribution" }
                            span { class: "section-card-meta", "labels per class, fewest first" }
                        }
                        ClassChart { plan, columns: columns(&p) }
                    }
                    section { class: "section-card", aria_labelledby: "sample-title",
                        h2 { id: "sample-title", class: "section-card-title", "Sample" }
                        div { class: "option-row",
                            label { class: "check",
                                input {
                                    r#type: "checkbox",
                                    checked: subset.is_some(),
                                    onchange: move |e| plan.write().subset_percent = e.checked().then_some(10),
                                }
                                span { "Random subset" }
                            }
                            if let Some(percent) = subset {
                                NumberInput { value: percent, label: "Subset in percent", onchange: move |n| plan.write().subset_percent = Some(n) }
                                span { class: "field-hint", "% of the images, taken from each split, before the threshold applies" }
                            }
                        }
                        SeedField { plan, used: p.seed, hint: "The same seed gives the same subset and the same cut again." }
                    }
                    ResultCard { preview: p.clone(), id: "balance-result-title" }
                    RunCard { plan: plan(), can_run: p.can_run(), name, suffix: "balanced", kind: JobKind::Balance, action: "Balance" }
                }
            },
            None if error().is_some() => rsx! {},
            None => rsx! { p { class: "field-hint", "Reading labels…" } },
        }
    }
}

/// One class in the chart: its labels before and after the plan.
#[derive(Debug, Clone, PartialEq)]
struct Column {
    name: String,
    before: u32,
    after: u32,
}

/// Classes that have labels, fewest first.
fn columns(preview: &BalancePreview) -> Vec<Column> {
    let mut columns: Vec<Column> = preview
        .classes
        .iter()
        .map(|c| Column { name: c.name.clone(), before: c.before.iter().sum(), after: c.after.iter().sum() })
        .filter(|c| c.before > 0)
        .collect();
    columns.sort_by_key(|c| c.before);
    columns
}

/// Tick step and top of a count axis that holds `largest`: about four steps
/// of 1, 2 or 5 times a power of ten.
fn axis_scale(largest: u32) -> (u32, u32) {
    let rough = (f64::from(largest) / 4.0).max(1.0);
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * magnitude).find(|step| *step >= rough).unwrap_or(rough);
    let step = (step as u32).max(1);
    (step, largest.div_ceil(step) * step)
}

/// Column chart of labels per class. Under each column a button includes or
/// leaves out the class. A dashed threshold line (drag it or its handle on the
/// axis, or use the arrow keys on the handle) cuts dominant classes down:
/// the part of a column that goes is drawn faded.
#[component]
fn ClassChart(plan: Signal<BalancePlan>, columns: Vec<Column>) -> Element {
    let mut plan = plan;
    let mut axis = use_signal(|| None::<Rc<MountedData>>);
    // While dragging: the threshold under the pointer, and the plot's top edge and height in px.
    let mut drag = use_signal(|| None::<(u32, f64, f64)>);

    let dropped = plan.read().drop_classes.clone();
    let cap = plan.read().class_cap;
    let largest = columns.iter().map(|c| c.before).max().unwrap_or(1);
    let (step, top) = axis_scale(largest);
    // Resting on the tallest column, the line cuts nothing.
    let threshold = drag().map(|d| d.0).or(cap).unwrap_or(largest).min(largest);
    let cutting = threshold < largest;
    let from_top = move |value: u32| 1.0 - f64::from(value) / f64::from(top);
    // Axis ticks and the line, as a share of the plot's height from its top.
    let ticks: Vec<(u32, f64)> = (0..=top / step).map(|i| (i * step, from_top(i * step))).collect();
    let line = from_top(threshold);
    let included = columns.iter().filter(|c| !dropped.contains(&c.name)).count();
    let all_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();

    let mut commit = move |value: u32| {
        let value = value.clamp(1, largest);
        plan.write().class_cap = (value < largest).then_some(value);
    };
    let start_drag = move |e: PointerEvent| {
        e.prevent_default();
        spawn(async move {
            let Some(element) = axis() else { return };
            if let Ok(rect) = element.get_client_rect().await {
                drag.set(Some((threshold, rect.origin.y, rect.size.height.max(1.0))));
            }
        });
    };
    let threshold_text = if cutting { format!("At most {} labels per class", format_count(threshold)) } else { "No limit".to_string() };

    rsx! {
        div { class: "class-chart",
            div { class: "class-chart-axis", onmounted: move |e| axis.set(Some(e.data())),
                for (tick, share) in ticks.iter() {
                    span { key: "{tick}", class: "class-chart-tick", style: "top: {share * 100.0:.3}%", aria_hidden: "true",
                        "{format_count(*tick)}"
                    }
                }
                div {
                    class: if drag().is_some() { "threshold-grip is-dragging" } else { "threshold-grip" },
                    style: "top: {line * 100.0:.3}%",
                    role: "slider",
                    tabindex: "0",
                    aria_label: "Threshold: most labels a class keeps",
                    aria_orientation: "vertical",
                    aria_valuemin: "1",
                    aria_valuemax: "{largest}",
                    aria_valuenow: "{threshold}",
                    aria_valuetext: "{threshold_text}",
                    onpointerdown: start_drag,
                    onkeydown: move |e| {
                        let page = (largest / 10).max(1);
                        match e.key() {
                            Key::ArrowUp | Key::ArrowRight => commit(threshold.saturating_add(1)),
                            Key::ArrowDown | Key::ArrowLeft => commit(threshold.saturating_sub(1)),
                            Key::PageUp => commit(threshold.saturating_add(page)),
                            Key::PageDown => commit(threshold.saturating_sub(page)),
                            Key::Home => commit(largest),
                            Key::End => commit(1),
                            _ => return,
                        }
                        e.prevent_default();
                    },
                }
            }
            div { class: "class-chart-scroll",
                div { class: "class-chart-inner",
                    for (tick, share) in ticks.iter() {
                        div { key: "{tick}", class: "class-chart-gridline", style: "top: calc(var(--plot-height) * {share:.4})" }
                    }
                    for column in columns.iter() {
                        {
                            let name = column.name.clone();
                            let is_in = !dropped.contains(&name);
                            // While the line is dragged, show its cut; the exact result follows on release.
                            let kept = match (is_in, drag()) {
                                (false, _) => 0,
                                (true, Some(_)) => column.before.min(threshold),
                                (true, None) => column.after,
                            };
                            let tip = if is_in {
                                format!("{name}: {} of {} labels kept", format_count(kept), format_count(column.before))
                            } else {
                                format!("{name}: left out ({} labels)", format_count(column.before))
                            };
                            let title = if is_in {
                                format!("{name}: included. Click to leave it out.")
                            } else {
                                format!("{name}: left out. Click to include it.")
                            };
                            let height = (1.0 - from_top(column.before)) * 100.0;
                            let kept_height = f64::from(kept) / f64::from(column.before) * 100.0;
                            rsx! {
                                div { key: "{name}", class: "class-col",
                                    div { class: "class-col-plot", "data-tip": "{tip}",
                                        div { class: "class-bar", style: "height: {height:.3}%",
                                            div { class: "class-bar-kept", style: "height: {kept_height:.3}%" }
                                        }
                                    }
                                    button {
                                        class: "class-toggle",
                                        r#type: "button",
                                        title: "{title}",
                                        aria_pressed: if is_in { "true" } else { "false" },
                                        onclick: {
                                            let name = name.clone();
                                            move |_| {
                                                let classes = &mut plan.write().drop_classes;
                                                if is_in {
                                                    classes.push(name.clone());
                                                } else {
                                                    classes.retain(|c| *c != name);
                                                }
                                            }
                                        },
                                        "{name}"
                                    }
                                }
                            }
                        }
                    }
                    div {
                        class: if cutting { "threshold-line is-cutting" } else { "threshold-line" },
                        style: "top: calc(var(--plot-height) * {line:.4})",
                        aria_hidden: "true",
                        onpointerdown: start_drag,
                        span { class: "threshold-label", "{threshold_text}" }
                    }
                }
            }
        }
        p { class: "field-hint",
            "Solid: labels that stay. Faded: labels that go. Drag the dashed line down to cut dominant classes: whole images are removed, and an image with a class still under the line stays, so a column can end a little above it."
        }
        div { class: "option-row",
            button {
                class: "button button-secondary button-sm",
                r#type: "button",
                disabled: included == columns.len(),
                onclick: move |_| plan.write().drop_classes.clear(),
                "Select all"
            }
            button {
                class: "button button-secondary button-sm",
                r#type: "button",
                disabled: included == 0,
                onclick: move |_| plan.write().drop_classes = all_names.clone(),
                "Select none"
            }
            span { class: "section-card-meta", aria_live: "polite", "{included} of {columns.len()} classes included" }
        }

        // While dragging, a full-screen layer catches the pointer (see `ResizeHandle`).
        if let Some((value, plot_top, height)) = drag() {
            div {
                class: "resize-overlay",
                style: "cursor: row-resize",
                onpointermove: move |e| {
                    let share = (plot_top + height - e.client_coordinates().y) / height;
                    let under_pointer = (share * f64::from(top)).round().clamp(1.0, f64::from(largest)) as u32;
                    if under_pointer != value {
                        drag.set(Some((under_pointer, plot_top, height)));
                    }
                },
                onpointerup: move |_| {
                    commit(value);
                    drag.set(None);
                },
                onpointerleave: move |_| {
                    commit(value);
                    drag.set(None);
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_the_count_axis() {
        assert_eq!(axis_scale(388), (100, 400));
        assert_eq!(axis_scale(3), (1, 3));
        assert_eq!(axis_scale(40), (10, 40));
        assert_eq!(axis_scale(1200), (500, 1500));
    }
}
