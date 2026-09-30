//! Toasts: short messages that pop up in the bottom right corner and go away
//! by themselves. Any component can show one:
//!
//! ```ignore
//! let toasts = use_toasts();
//! toasts.info("Hugging Face downloads can be slow.");
//! toasts.show(ToastKind::Success, "Settings saved");
//! ```
//!
//! `AppLayout` calls `provide_toasts()` once and renders `ToastHost`.

use dioxus::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)] // Warning has no user yet
pub enum ToastKind {
    /// A hint or something to know.
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
struct Toast {
    id: u64,
    kind: ToastKind,
    text: String,
}

/// Handle to the app's toasts; get it with `use_toasts()`.
#[derive(Clone, Copy)]
pub struct Toasts {
    items: Signal<Vec<Toast>>,
    next_id: Signal<u64>,
}

impl Toasts {
    /// Shows `text`. A message that is already showing isn't shown twice.
    pub fn show(mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        if self.items.peek().iter().any(|t| t.text == text) {
            return;
        }
        let id = *self.next_id.peek();
        self.next_id.set(id + 1);
        self.items.write().push(Toast { id, kind, text });
    }

    pub fn info(self, text: impl Into<String>) {
        self.show(ToastKind::Info, text);
    }

    fn dismiss(mut self, id: u64) {
        self.items.write().retain(|t| t.id != id);
    }
}

/// Call once near the root, above everything that shows toasts.
pub fn provide_toasts() {
    use_context_provider(|| Toasts { items: Signal::new(Vec::new()), next_id: Signal::new(0) });
}

pub fn use_toasts() -> Toasts {
    use_context::<Toasts>()
}

/// The stack of visible toasts. Each one leaves when its CSS animation ends
/// (`toast-life` in main.css; hovering or focusing it pauses that), or when
/// its close button is pressed. No timers are needed this way.
#[component]
pub fn ToastHost() -> Element {
    let toasts = use_toasts();

    rsx! {
        div { class: "toast-host", role: "region", aria_label: "Messages", aria_live: "polite",
            for toast in toasts.items.read().iter().cloned() {
                {
                    let (class, label) = match toast.kind {
                        ToastKind::Info => ("toast is-info", "Tip"),
                        ToastKind::Success => ("toast is-success", "Done"),
                        ToastKind::Warning => ("toast is-warning", "Warning"),
                        ToastKind::Error => ("toast is-error", "Error"),
                    };
                    let id = toast.id;
                    rsx! {
                        div {
                            key: "{id}",
                            class,
                            role: if toast.kind == ToastKind::Error { "alert" } else { "status" },
                            onanimationend: move |_| toasts.dismiss(id),
                            span { class: "toast-kind", "{label}" }
                            p { class: "toast-text", "{toast.text}" }
                            button {
                                class: "icon-button icon-button-sm toast-close",
                                r#type: "button",
                                aria_label: "Dismiss",
                                onclick: move |_| toasts.dismiss(id),
                                "×"
                            }
                        }
                    }
                }
            }
        }
    }
}
