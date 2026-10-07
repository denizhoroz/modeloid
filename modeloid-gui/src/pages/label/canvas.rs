//! The annotation view: the image with its labels, zoomed and panned. One
//! transparent layer on top takes every pointer event, so positions are
//! always relative to the stage; hit testing is done in `geometry`.

use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use super::geometry::{self, Handle, Point, View};
use super::{use_editor, Tool};
use crate::api::explore::image_url;
use crate::api::label::LabelObject;
use crate::components::use_toasts;

/// Pointer distance (CSS px) that still grabs a handle.
const HANDLE_REACH: f64 = 7.0;
/// Drawn handle size (CSS px).
const HANDLE_SIZE: f64 = 8.0;

#[derive(Debug, Clone, PartialEq)]
enum Drag {
    /// A new box from `start` to `now`.
    Draw { start: Point, now: Point },
    /// The selected labels, from where they were (`before`).
    Move { start: Point, before: Vec<LabelObject> },
    Resize { index: usize, handle: Handle, before: Vec<LabelObject> },
    /// The view, from stage point `start`.
    Pan { start: Point, view: View },
}

#[component]
pub fn Canvas() -> Element {
    let editor = use_editor();
    let toasts = use_toasts();
    let mut drag = use_signal(|| None::<Drag>);
    // Last pointer position on the stage (wheel events have none).
    let mut pointer = use_signal(|| None::<Point>);
    let mut cursor = use_signal(|| "default");

    let Some(image) = editor.image.read().clone() else { return rsx! {} };
    let size = [image.width as f64, image.height as f64];
    let view = (editor.view)();
    let objects = editor.objects.read().clone();
    let selected = editor.selected.read().clone();
    let classes = editor.classes();
    let tool = (editor.tool)();
    let panning = tool == Tool::Pan || (editor.space_pan)();
    let read_only = editor.read_only();

    let stage_point = |e: &PointerEvent| -> Point {
        let p = e.element_coordinates();
        [p.x, p.y]
    };

    let on_down = move |e: PointerEvent| {
        let stage = stage_point(&e);
        let view = (editor.view)();
        let p = view.to_image(stage);
        let button = e.trigger_button();
        if panning || button == Some(MouseButton::Auxiliary) {
            drag.set(Some(Drag::Pan { start: stage, view }));
            cursor.set("grabbing");
            return;
        }
        if button != Some(MouseButton::Primary) {
            return;
        }
        let mut selected = editor.selected;
        let objects = editor.objects.peek().clone();
        match (editor.tool)() {
            Tool::Box => {
                if read_only {
                    return;
                }
                if editor.classes().is_empty() {
                    toasts.info("Add a class in the Classes panel first.");
                    return;
                }
                let start = geometry::rect_between(p, p, size);
                drag.set(Some(Drag::Draw { start: [start[0], start[1]], now: [start[0], start[1]] }));
            }
            Tool::Select => {
                let reach = HANDLE_REACH / view.scale;
                if let (false, Some((index, handle))) = (read_only, geometry::hit_handle(&objects, &selected.peek(), p, reach)) {
                    drag.set(Some(Drag::Resize { index, handle, before: objects }));
                    return;
                }
                match geometry::hit_object(&objects, p) {
                    Some(i) if e.modifiers().shift() => {
                        let next = crate::components::table::toggled(&selected.peek(), &i, !selected.peek().contains(&i));
                        selected.set(next);
                    }
                    Some(i) => {
                        if !selected.peek().contains(&i) {
                            selected.set(vec![i]);
                        }
                        if !read_only {
                            drag.set(Some(Drag::Move { start: p, before: objects }));
                        }
                    }
                    None => {
                        if !e.modifiers().shift() {
                            selected.set(Vec::new());
                        }
                        drag.set(Some(Drag::Pan { start: stage, view }));
                    }
                }
            }
            Tool::Pan => {}
        }
    };

    let on_move = move |e: PointerEvent| {
        let stage = stage_point(&e);
        pointer.set(Some(stage));
        let view = (editor.view)();
        let p = view.to_image(stage);
        let (mut objects, mut view_signal, mut auto_fit) = (editor.objects, editor.view, editor.auto_fit);
        let current = drag.peek().clone();
        match current {
            Some(Drag::Draw { start, .. }) => {
                let b = geometry::rect_between(start, p, size);
                let now = [if p[0] < start[0] { b[0] } else { b[0] + b[2] }, if p[1] < start[1] { b[1] } else { b[1] + b[3] }];
                drag.set(Some(Drag::Draw { start, now }));
            }
            Some(Drag::Move { start, before }) => {
                let delta = [p[0] - start[0], p[1] - start[1]];
                objects.set(geometry::moved(&before, &editor.selected.peek(), delta, size));
            }
            Some(Drag::Resize { index, handle, before }) => {
                let mut next = before.clone();
                if let Some(o) = next.get_mut(index) {
                    o.bbox = geometry::resized(before[index].bbox, handle, p, size);
                }
                objects.set(next);
            }
            Some(Drag::Pan { start, view: from }) => {
                view_signal.set(View { scale: from.scale, x: from.x + stage[0] - start[0], y: from.y + stage[1] - start[1] });
                auto_fit.set(false);
            }
            None => {
                // What a press here would do.
                let objects = editor.objects.peek();
                let next = if panning {
                    "grab"
                } else {
                    match (editor.tool)() {
                        Tool::Box => "crosshair",
                        Tool::Pan => "grab",
                        Tool::Select => match geometry::hit_handle(&objects, &editor.selected.peek(), p, HANDLE_REACH / view.scale) {
                            Some((_, handle)) if !read_only => handle.cursor(),
                            _ if geometry::hit_object(&objects, p).is_some() => "move",
                            _ => "default",
                        },
                    }
                };
                if *cursor.peek() != next {
                    cursor.set(next);
                }
            }
        }
    };

    let mut finish = move || {
        let Some(done) = drag.take() else { return };
        let mut selected = editor.selected;
        match done {
            Drag::Draw { start, now } => {
                let bbox = geometry::rect_between(start, now, size);
                if geometry::big_enough(bbox) {
                    let class = *editor.active_class.peek();
                    let index = editor.objects.peek().len();
                    editor.edit(|objects| objects.push(LabelObject { class, bbox, polygon: None }));
                    selected.set(vec![index]);
                }
            }
            Drag::Move { before, .. } | Drag::Resize { before, .. } => {
                if *editor.objects.peek() != before {
                    editor.commit(before);
                }
            }
            Drag::Pan { .. } => cursor.set(if panning { "grab" } else { "default" }),
        }
    };

    let draft = match &*drag.read() {
        Some(Drag::Draw { start, now }) => Some(geometry::rect_between(*start, *now, size)),
        _ => None,
    };
    let handle_size = HANDLE_SIZE / view.scale;
    let show_labels = (editor.show_labels)();
    let name_of = |c: usize| classes.get(c).cloned().unwrap_or_else(|| format!("class {c}"));
    let (frame_w, frame_h) = (size[0] * view.scale, size[1] * view.scale);

    rsx! {
        div {
            class: "label-stage",
            onresize: move |e| {
                if let Ok(s) = e.get_content_box_size() {
                    let mut stage = editor.stage;
                    stage.set([s.width, s.height]);
                    if *editor.auto_fit.peek() {
                        editor.fit();
                    }
                }
            },
            div {
                class: "label-frame",
                style: "left: {view.x}px; top: {view.y}px; width: {frame_w}px; height: {frame_h}px",
                img { src: image_url(editor.id, image.index), alt: "{image.name}", draggable: "false" }
                svg {
                    class: "label-overlay",
                    view_box: "0 0 {size[0]} {size[1]}",
                    preserve_aspect_ratio: "none",
                    "aria-hidden": "true",
                    if show_labels {
                        for (i, o) in objects.iter().enumerate() {
                            {
                                let class = if selected.contains(&i) { "label-shape is-selected" } else { "label-shape" };
                                let style = format!("--label-color: {}", editor.color(o.class));
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
                    for &i in selected.iter().filter(|&&i| !read_only && objects.get(i).is_some_and(geometry::resizable)) {
                        for handle in Handle::ALL {
                            {
                                let [hx, hy] = handle.at(objects[i].bbox);
                                rsx! {
                                    rect {
                                        key: "{i}-{handle:?}",
                                        class: "label-handle",
                                        x: "{hx - handle_size / 2.0}",
                                        y: "{hy - handle_size / 2.0}",
                                        width: "{handle_size}",
                                        height: "{handle_size}",
                                    }
                                }
                            }
                        }
                    }
                    if let Some(b) = draft {
                        rect {
                            class: "label-draft",
                            style: "--label-color: {editor.color(*editor.active_class.read())}",
                            x: "{b[0]}",
                            y: "{b[1]}",
                            width: "{b[2]}",
                            height: "{b[3]}",
                        }
                    }
                }
                if show_labels {
                    for &i in selected.iter() {
                        if let Some(o) = objects.get(i) {
                            span {
                                key: "{i}",
                                class: "box-tag label-tag",
                                style: "left: {o.bbox[0] / size[0] * 100.0:.3}%; top: {o.bbox[1] / size[1] * 100.0:.3}%; background: {editor.color(o.class)}",
                                "{name_of(o.class)}"
                            }
                        }
                    }
                }
            }
            div {
                class: "label-hit",
                style: "cursor: {cursor}",
                onpointerdown: on_down,
                onpointermove: on_move,
                onpointerup: move |_| finish(),
                onpointerleave: move |_| {
                    pointer.set(None);
                    finish();
                },
                oncontextmenu: move |e| e.prevent_default(),
                onwheel: move |e| {
                    let dy = e.delta().strip_units().y;
                    if dy != 0.0 {
                        editor.zoom(if dy < 0.0 { 1.15 } else { 1.0 / 1.15 }, *pointer.peek());
                    }
                },
            }
        }
    }
}
