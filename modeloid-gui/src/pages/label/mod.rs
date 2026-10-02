//! Labelling interface (`/label/:id/:index`): draw and edit boxes on a
//! dataset's images, one image at a time. Tools on the left, the image in the
//! middle with a bar above it, the Classes table on the right.
//!
//! Edits stay in the page until they're saved: 1.5 s after the last change,
//! before moving to another image, on Ctrl+S, and when leaving the page.

mod canvas;
mod classes;
mod geometry;
mod tools;

use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use dioxus::router::Navigator;
use modeloid_types::Split;

use crate::api::error_message;
use crate::api::explore::image_url;
use crate::api::label::{label_image, label_session, save_labels, LabelImage, LabelObject, LabelSession};
use crate::components::{Icon, IconName, PanelWidth, ResizeHandle};
use crate::routes::Route;
use crate::util::format_count;

use canvas::Canvas;
use classes::ClassesPanel;
use geometry::{Point, View};
use tools::ToolsPanel;

/// Box colours by class: bright, so they show on photos in every theme.
const CLASS_COLORS: [&str; 12] = [
    "#ff6b6b", "#4dabf7", "#51cf66", "#fcc419", "#cc5de8", "#ff922b", "#22b8cf", "#f06595", "#94d82d", "#845ef7", "#20c997", "#ffd43b",
];

pub fn class_color(class: usize) -> &'static str {
    CLASS_COLORS[class % CLASS_COLORS.len()]
}

/// Set before going back to Datasets, so it opens on the Edit tab.
pub static RETURN_TO_EDIT: GlobalSignal<bool> = Signal::global(|| false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Select,
    Box,
    Pan,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SaveState {
    Saved,
    Saving,
    Failed(String),
}

/// The page's editing state, shared with its parts through context. Every
/// field is a signal, so it's `Copy`.
#[derive(Clone, Copy)]
pub struct Editor {
    pub id: u32,
    nav: Navigator,
    /// Classes, counts and which images have labels; `None` while loading.
    pub session: Signal<Option<LabelSession>>,
    /// The open image (its `objects` as loaded; the edited ones are in `objects`).
    pub image: Signal<Option<LabelImage>>,
    pub objects: Signal<Vec<LabelObject>>,
    undo: Signal<Vec<Vec<LabelObject>>>,
    redo: Signal<Vec<Vec<LabelObject>>>,
    /// Indexes into `objects`.
    pub selected: Signal<Vec<usize>>,
    pub tool: Signal<Tool>,
    /// Class of new boxes.
    pub active_class: Signal<usize>,
    /// Changed since the last save.
    pub dirty: Signal<bool>,
    /// Counts changes, so a save knows whether more came in meanwhile.
    pub edits: Signal<u64>,
    pub save_state: Signal<SaveState>,
    pub view: Signal<View>,
    /// Refit the image when the stage changes size, until the user zooms or pans.
    pub auto_fit: Signal<bool>,
    /// Size of the stage the image sits in, in CSS pixels.
    pub stage: Signal<Point>,
    pub show_labels: Signal<bool>,
    /// Previous / Next skip images that have labels.
    pub only_unlabelled: Signal<bool>,
    clipboard: Signal<Vec<LabelObject>>,
    /// Space held: dragging pans.
    pub space_pan: Signal<bool>,
    /// Bumped to load the image / session again.
    pub image_version: Signal<u64>,
    pub session_version: Signal<u64>,
}

pub fn use_editor() -> Editor {
    use_context::<Editor>()
}

impl Editor {
    fn new(id: u32, nav: Navigator) -> Editor {
        Editor {
            id,
            nav,
            session: Signal::new(None),
            image: Signal::new(None),
            objects: Signal::new(Vec::new()),
            undo: Signal::new(Vec::new()),
            redo: Signal::new(Vec::new()),
            selected: Signal::new(Vec::new()),
            tool: Signal::new(Tool::Box),
            active_class: Signal::new(0),
            dirty: Signal::new(false),
            edits: Signal::new(0),
            save_state: Signal::new(SaveState::Saved),
            view: Signal::new(View { scale: 1.0, x: 0.0, y: 0.0 }),
            auto_fit: Signal::new(true),
            stage: Signal::new([0.0, 0.0]),
            show_labels: Signal::new(true),
            only_unlabelled: Signal::new(false),
            clipboard: Signal::new(Vec::new()),
            space_pan: Signal::new(false),
            image_version: Signal::new(0),
            session_version: Signal::new(0),
        }
    }

    /// A job works on the dataset: nothing can be changed.
    pub fn read_only(&self) -> bool {
        self.session.read().as_ref().is_some_and(|s| s.busy.is_some())
    }

    pub fn classes(&self) -> Vec<String> {
        self.session.read().as_ref().map(|s| s.classes.clone()).unwrap_or_default()
    }

    /// Changes the labels with `change`, undoably.
    pub fn edit(mut self, change: impl FnOnce(&mut Vec<LabelObject>)) {
        if self.read_only() {
            return;
        }
        let before = self.objects.peek().clone();
        self.objects.with_mut(change);
        if *self.objects.peek() != before {
            self.commit(before);
        }
    }

    /// Records a change already made to `objects` (e.g. by a drag), undoably.
    pub fn commit(mut self, before: Vec<LabelObject>) {
        self.undo.write().push(before);
        self.redo.write().clear();
        self.changed();
    }

    fn changed(mut self) {
        self.dirty.set(true);
        *self.edits.write() += 1;
    }

    pub fn undo(mut self) {
        let Some(previous) = self.undo.write().pop() else { return };
        let current = self.objects.replace(previous);
        self.redo.write().push(current);
        self.selected.set(Vec::new());
        self.changed();
    }

    pub fn redo(mut self) {
        let Some(next) = self.redo.write().pop() else { return };
        let current = self.objects.replace(next);
        self.undo.write().push(current);
        self.selected.set(Vec::new());
        self.changed();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.read().is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.read().is_empty()
    }

    pub fn delete_selected(mut self) {
        let selected = self.selected.peek().clone();
        if selected.is_empty() {
            return;
        }
        self.edit(|objects| {
            let mut i = 0;
            objects.retain(|_| {
                i += 1;
                !selected.contains(&(i - 1))
            });
        });
        self.selected.set(Vec::new());
    }

    /// Makes `class` the class of new boxes and of the selected ones.
    pub fn choose_class(mut self, class: usize) {
        if class >= self.classes().len() {
            return;
        }
        self.active_class.set(class);
        let selected = self.selected.peek().clone();
        self.edit(|objects| {
            for &i in &selected {
                if let Some(o) = objects.get_mut(i) {
                    o.class = class;
                }
            }
        });
    }

    /// Moves the selected labels by `delta` image pixels.
    pub fn nudge(self, delta: Point) {
        let Some(image) = self.image.peek().clone() else { return };
        let selected = self.selected.peek().clone();
        let size = [image.width as f64, image.height as f64];
        self.edit(|objects| *objects = geometry::moved(objects, &selected, delta, size));
    }

    pub fn select_all(mut self) {
        let n = self.objects.peek().len();
        self.selected.set((0..n).collect());
    }

    pub fn copy(mut self) {
        let objects = self.objects.peek();
        let copied: Vec<LabelObject> = self.selected.peek().iter().filter_map(|&i| objects.get(i).cloned()).collect();
        if !copied.is_empty() {
            self.clipboard.set(copied);
        }
    }

    /// Adds the copied labels (kept inside this image), selected.
    pub fn paste(mut self) {
        let Some(image) = self.image.peek().clone() else { return };
        let (w, h) = (image.width as f64, image.height as f64);
        let pasted: Vec<LabelObject> = self
            .clipboard
            .peek()
            .iter()
            .map(|o| {
                let b = geometry::rect_between([o.bbox[0], o.bbox[1]], [o.bbox[0] + o.bbox[2], o.bbox[1] + o.bbox[3]], [w, h]);
                LabelObject { class: o.class, bbox: b, polygon: o.polygon.clone() }
            })
            .filter(|o| geometry::big_enough(o.bbox))
            .collect();
        if pasted.is_empty() {
            return;
        }
        let first = self.objects.peek().len();
        let count = pasted.len();
        self.edit(|objects| objects.extend(pasted));
        self.selected.set((first..first + count).collect());
    }

    /// Shows a freshly loaded image with its labels.
    fn load(mut self, image: LabelImage) {
        self.objects.set(image.objects.clone());
        self.image.set(Some(image));
        self.undo.write().clear();
        self.redo.write().clear();
        self.selected.set(Vec::new());
        self.dirty.set(false);
        if !matches!(*self.save_state.peek(), SaveState::Saved) {
            self.save_state.set(SaveState::Saved);
        }
        self.auto_fit.set(true);
        self.fit();
    }

    /// Fits the image into the stage.
    pub fn fit(mut self) {
        let stage = *self.stage.peek();
        if stage[0] <= 0.0 || stage[1] <= 0.0 {
            return;
        }
        if let Some(image) = self.image.peek().as_ref() {
            self.view.set(View::fit(stage, [image.width as f64, image.height as f64]));
            self.auto_fit.set(true);
        }
    }

    /// Zooms by `factor` around stage point `at` (the middle when `None`).
    pub fn zoom(mut self, factor: f64, at: Option<Point>) {
        let stage = *self.stage.peek();
        let at = at.unwrap_or([stage[0] / 2.0, stage[1] / 2.0]);
        let view = self.view.peek().zoomed(factor, at);
        self.view.set(view);
        self.auto_fit.set(false);
    }

    /// Saves the open image's labels if they changed; false when that failed.
    pub async fn save(mut self) -> bool {
        if !*self.dirty.peek() {
            return true;
        }
        let (Some(session), Some(image)) = (self.session.peek().clone(), self.image.peek().clone()) else { return false };
        let objects = self.objects.peek().clone();
        let seq = *self.edits.peek();
        self.save_state.set(SaveState::Saving);
        match save_labels(self.id, session.dataset, image.index, image.name, objects).await {
            Ok(saved) => {
                if let Some(session) = self.session.write().as_mut() {
                    session.totals = saved.totals;
                    if let Some(labelled) = session.labelled.get_mut(image.index) {
                        *labelled = saved.labelled;
                    }
                }
                // Changes made while saving still need a save.
                if *self.edits.peek() == seq {
                    self.dirty.set(false);
                }
                self.save_state.set(SaveState::Saved);
                true
            }
            Err(e) => {
                self.save_state.set(SaveState::Failed(error_message(&e)));
                false
            }
        }
    }

    /// Saves, then opens image `index`.
    pub async fn go(self, index: usize) {
        if self.save().await {
            self.nav.replace(Route::Label { id: self.id, index });
        }
    }

    /// The image `step` away from the open one, skipping labelled ones when
    /// `only_unlabelled` is on.
    pub fn neighbour(&self, step: isize) -> Option<usize> {
        let session = self.session.read();
        let session = session.as_ref()?;
        let current = self.image.read().as_ref()?.index;
        let total = session.labelled.len() as isize;
        let only_unlabelled = (self.only_unlabelled)();
        let mut i = current as isize + step;
        while i >= 0 && i < total {
            if !only_unlabelled || !session.labelled[i as usize] {
                return Some(i as usize);
            }
            i += step;
        }
        None
    }

    pub async fn step(self, step: isize) {
        if let Some(next) = self.neighbour(step) {
            self.go(next).await;
        }
    }

    /// Saves and goes back to Datasets > Edit.
    pub async fn close(self) {
        if self.save().await {
            *RETURN_TO_EDIT.write() = true;
            self.nav.push(Route::Tool { section: "dataset".into(), tool: "datasets".into() });
        }
    }
}

/// The route's page. A new dataset starts with fresh state.
#[component]
pub fn Label(id: u32, index: usize) -> Element {
    rsx! { LabelPage { key: "{id}", id, index } }
}

#[component]
fn LabelPage(id: u32, index: ReadSignal<usize>) -> Element {
    let nav = use_navigator();
    let editor = use_context_provider(|| Editor::new(id, nav));
    let mut root = use_signal(|| None::<std::rc::Rc<MountedData>>);
    let panel_width = use_context::<PanelWidth>().0;

    let session = use_resource(move || async move {
        let _ = (editor.session_version)();
        label_session(id).await
    });
    let image = use_resource(move || async move {
        let _ = (editor.image_version)();
        label_image(id, index()).await
    });

    use_effect(move || {
        if let Some(Ok(loaded)) = &*session.read() {
            let mut editor = editor;
            let classes = loaded.classes.len();
            editor.session.set(Some(loaded.clone()));
            if *editor.active_class.peek() >= classes {
                editor.active_class.set(classes.saturating_sub(1));
            }
            // An index past the end (images removed meanwhile): the last image.
            let total = loaded.labelled.len();
            if total > 0 && index() >= total {
                nav.replace(Route::Label { id, index: total - 1 });
            }
        }
    });
    use_effect(move || {
        if let Some(Ok(loaded)) = &*image.read() {
            editor.load(loaded.clone());
        }
    });

    // Autosave a moment after the last change.
    use_effect(move || {
        let seq = (editor.edits)();
        if seq == 0 {
            return;
        }
        spawn(async move {
            let _ = document::eval("await new Promise(r => setTimeout(r, 1500));").await;
            if *editor.edits.peek() == seq && *editor.dirty.peek() {
                editor.save().await;
            }
        });
    });

    // Leaving the page (a sidebar link): save what's left, outliving the page.
    use_drop(move || {
        let dirty = editor.dirty.try_peek().map(|d| *d).unwrap_or(false);
        let session = editor.session.try_peek().ok().and_then(|s| s.clone());
        let image = editor.image.try_peek().ok().and_then(|i| i.clone());
        let objects = editor.objects.try_peek().map(|o| o.clone()).unwrap_or_default();
        if let (true, Some(session), Some(image)) = (dirty, session, image) {
            spawn_forever(async move {
                let _ = save_labels(id, session.dataset, image.index, image.name, objects).await;
            });
        }
    });

    let focus = move || {
        if let Some(root) = root.peek().clone() {
            spawn(async move {
                let _ = root.set_focus(true).await;
            });
        }
    };

    let name = editor.session.read().as_ref().map(|s| s.dataset.clone());
    let title = match &name {
        Some(name) => format!("Labelling · {name} · Modeloid"),
        None => "Labelling · Modeloid".to_string(),
    };
    let total = editor.session.read().as_ref().map_or(0, |s| s.labelled.len());
    let next = editor.neighbour(1);

    rsx! {
        document::Title { "{title}" }
        div {
            class: "label-page",
            style: "--panel-width: {panel_width}px",
            tabindex: "-1",
            onmounted: move |e| async move {
                let data = e.data();
                root.set(Some(data.clone()));
                let _ = data.set_focus(true).await;
            },
            onkeydown: move |e| on_key(editor, e),
            onkeyup: move |e| {
                if e.key() == Key::Character(" ".into()) {
                    let mut space = editor.space_pan;
                    space.set(false);
                }
            },
            ToolsPanel {}
            div { class: "label-main",
                TopBar {}
                if let Some(reason) = editor.session.read().as_ref().and_then(|s| s.busy.clone()) {
                    p { class: "notice notice-warning label-notice", role: "status", "Read only: {reason}" }
                }
                match (&*session.read(), &*image.read()) {
                    (Some(Err(e)), _) => rsx! { LoadError { message: error_message(e) } },
                    (Some(Ok(_)), _) if total == 0 => rsx! {
                        div { class: "label-empty",
                            p { "This dataset has no images yet. Add some in Datasets > Edit, then open labelling again." }
                            button { class: "button", r#type: "button", onclick: move |_| async move { editor.close().await }, "Back to Datasets" }
                        }
                    },
                    (_, Some(Err(e))) => rsx! { LoadError { message: error_message(e) } },
                    (Some(Ok(_)), Some(Ok(_))) => rsx! {
                        div { class: "label-stage-wrap", onpointerdown: move |_| focus(),
                            Canvas {}
                        }
                    },
                    _ => rsx! { p { class: "label-empty", aria_live: "polite", "Loading…" } },
                }
                if let Some(next) = next {
                    // The next image loads while this one is labelled.
                    link { rel: "prefetch", href: image_url(id, next) }
                }
            }
            ResizeHandle {
                width: panel_width,
                min: 260.0,
                max: 900.0,
                label: "Resize classes panel",
                controls: "label-classes",
            }
            ClassesPanel {}
        }
    }
}

#[component]
fn LoadError(message: String) -> Element {
    let editor = use_editor();
    rsx! {
        div { class: "label-empty",
            p { class: "form-error", role: "alert", "{message}" }
            button { class: "button", r#type: "button", onclick: move |_| async move { editor.close().await }, "Back to Datasets" }
        }
    }
}

/// Keyboard shortcuts of the page (inputs stop their keys from reaching here).
fn on_key(editor: Editor, e: KeyboardEvent) {
    let mods = e.modifiers();
    let (ctrl, shift) = (mods.ctrl() || mods.meta(), mods.shift());
    let mut handled = true;
    let (mut tool, mut show_labels, mut selected, mut space) = (editor.tool, editor.show_labels, editor.selected, editor.space_pan);
    match e.key() {
        Key::Character(c) => match (ctrl, c.to_lowercase().as_str()) {
            (true, "z") if shift => editor.redo(),
            (true, "z") => editor.undo(),
            (true, "y") => editor.redo(),
            (true, "s") => {
                spawn(async move {
                    editor.save().await;
                });
            }
            (true, "c") => editor.copy(),
            (true, "v") => editor.paste(),
            (true, "a") => editor.select_all(),
            (false, "v") => tool.set(Tool::Select),
            (false, "b") => tool.set(Tool::Box),
            (false, "h") => tool.set(Tool::Pan),
            (false, "l") => show_labels.toggle(),
            (false, "a") => {
                spawn(async move { editor.step(-1).await });
            }
            (false, "d") => {
                spawn(async move { editor.step(1).await });
            }
            (false, "+" | "=") => editor.zoom(1.25, None),
            (false, "-") => editor.zoom(1.0 / 1.25, None),
            (false, "0") => editor.fit(),
            (false, " ") => space.set(true),
            (false, digit) if digit.len() == 1 && ("1"..="9").contains(&digit) => {
                editor.choose_class(digit.parse::<usize>().unwrap_or(1) - 1);
            }
            _ => handled = false,
        },
        Key::Delete | Key::Backspace => editor.delete_selected(),
        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown => {
            let amount = if shift { 10.0 } else { 1.0 };
            let delta = match e.key() {
                Key::ArrowLeft => [-amount, 0.0],
                Key::ArrowRight => [amount, 0.0],
                Key::ArrowUp => [0.0, -amount],
                _ => [0.0, amount],
            };
            if !selected.peek().is_empty() {
                editor.nudge(delta);
            } else if delta[1] == 0.0 {
                let step = if delta[0] < 0.0 { -1 } else { 1 };
                spawn(async move { editor.step(step).await });
            }
        }
        Key::Escape => {
            if !selected.peek().is_empty() {
                selected.set(Vec::new());
            } else {
                spawn(async move { editor.close().await });
            }
        }
        _ => handled = false,
    }
    if handled {
        e.prevent_default();
    }
}

/// Above the image: its name and state, the position (type a number to jump),
/// the filter, Previous / Next and Close.
#[component]
fn TopBar() -> Element {
    let editor = use_editor();
    let mut only_unlabelled = editor.only_unlabelled;
    let image = editor.image.read().clone();
    let total = editor.session.read().as_ref().map_or(0, |s| s.labelled.len());
    let labelled = !editor.objects.read().is_empty();
    let (prev, next) = (editor.neighbour(-1), editor.neighbour(1));
    let mut typed = use_signal(String::new);
    let position = image.as_ref().map_or(0, |i| i.index + 1);
    use_effect(use_reactive((&position,), move |(position,)| typed.set(position.to_string())));

    let mut jump = move || {
        if let Ok(n) = typed.peek().trim().parse::<usize>() {
            if n >= 1 && n <= total {
                spawn(async move { editor.go(n - 1).await });
                return;
            }
        }
        typed.set(position.to_string());
    };

    let save_label = match (editor.save_state)() {
        SaveState::Saving => rsx! { span { class: "label-save", "Saving…" } },
        SaveState::Failed(message) => rsx! {
            span { class: "label-save is-failed", title: "{message}",
                "Save failed · "
                button { class: "link-button", r#type: "button", onclick: move |_| async move { editor.save().await; }, "Retry" }
                " · "
                button {
                    class: "link-button",
                    r#type: "button",
                    title: "Drop the unsaved changes of this image",
                    onclick: move |_| {
                        let mut version = editor.image_version;
                        *version.write() += 1;
                    },
                    "Discard"
                }
            }
        },
        SaveState::Saved if (editor.dirty)() => rsx! { span { class: "label-save", "Unsaved" } },
        SaveState::Saved => rsx! { span { class: "label-save is-saved", "Saved" } },
    };

    rsx! {
        div { class: "label-topbar", role: "toolbar", aria_label: "Image",
            div { class: "label-topbar-info",
                if let Some(image) = &image {
                    span { class: "label-image-name", title: "{image.name}", "{image.name}" }
                    if image.split.is_some() {
                        span { class: "tag", "{Split::label(image.split)}" }
                    }
                    span { class: if labelled { "tag tag-ok" } else { "tag" }, if labelled { "labelled" } else { "not labelled" } }
                }
                {save_label}
            }
            div { class: "label-topbar-position",
                input {
                    class: "input label-jump",
                    r#type: "text",
                    inputmode: "numeric",
                    aria_label: "Image number",
                    value: "{typed}",
                    oninput: move |e| typed.set(e.value()),
                    onkeydown: move |e| {
                        e.stop_propagation();
                        if e.key() == Key::Enter {
                            jump();
                        }
                    },
                    onblur: move |_| jump(),
                }
                span { class: "label-total", "/ {format_count(total as u32)}" }
            }
            div { class: "label-topbar-actions",
                select {
                    class: "input select label-filter",
                    aria_label: "Which images Previous and Next go to",
                    onchange: move |e| only_unlabelled.set(e.value() == "unlabelled"),
                    onkeydown: move |e| e.stop_propagation(),
                    option { value: "all", selected: !only_unlabelled(), "All images" }
                    option { value: "unlabelled", selected: only_unlabelled(), "Not labelled" }
                }
                button {
                    class: "icon-button",
                    r#type: "button",
                    aria_label: "Previous image",
                    title: "Previous image (A)",
                    disabled: prev.is_none(),
                    onclick: move |_| async move { editor.step(-1).await },
                    Icon { name: IconName::ChevronLeft, size: 18 }
                }
                button {
                    class: "icon-button",
                    r#type: "button",
                    aria_label: "Next image",
                    title: "Next image (D)",
                    disabled: next.is_none(),
                    onclick: move |_| async move { editor.step(1).await },
                    Icon { name: IconName::ChevronRight, size: 18 }
                }
                button {
                    class: "button button-secondary button-sm",
                    r#type: "button",
                    title: "Save and go back to Datasets > Edit",
                    onclick: move |_| async move { editor.close().await },
                    "Close"
                }
            }
        }
    }
}
