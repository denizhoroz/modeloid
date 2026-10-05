use dioxus::prelude::*;
use modeloid_types::formats::{self, FormatTask, FORMATS};

/// Select value of Download's "Other…" option (a typed export name).
pub const OTHER_FORMAT: &str = "__other";

/// What a `FormatSelect` picks a format for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatUse {
    /// A Roboflow export: every format Roboflow offers (and "Other…");
    /// formats Modeloid can't read yet say so.
    Download,
    /// Labels Modeloid writes: every format, those it can't write yet
    /// disabled.
    Write,
}

/// Annotation format dropdown, built from the one format list
/// (`modeloid_types::formats`) and grouped by task.
///
/// `value` may be a format id ("yolo26") or a family name ("yolo"); a family
/// name shows as the family's first format. `current` (a format or family
/// name) marks the input's own format with "(current)".
///
/// ```ignore
/// FormatSelect { id: "convert-format", value: plan().format, use_for: FormatUse::Write, onchange: move |v| plan.write().format = v }
/// ```
#[component]
pub fn FormatSelect(
    #[props(into)] id: String,
    #[props(into)] value: String,
    use_for: FormatUse,
    onchange: EventHandler<String>,
    /// A first option meaning "no format", e.g. "No default (follow each dataset)".
    #[props(into)]
    none_label: Option<String>,
    #[props(into)] current: Option<String>,
) -> Element {
    let shown = if value == OTHER_FORMAT { None } else { formats::resolve(&value).map(|f| f.id) };
    let current = current.as_deref().and_then(formats::resolve).map(|f| f.id);
    let download = use_for == FormatUse::Download;

    rsx! {
        select {
            id: "{id}",
            class: "input select",
            onchange: move |e| onchange.call(e.value()),
            if let Some(none) = none_label {
                option { value: "", selected: value.is_empty(), "{none}" }
            }
            for task in FormatTask::ALL {
                optgroup { key: "{task.label()}", label: task.label(),
                    for format in FORMATS.iter().filter(|f| f.task == task && (!download || f.roboflow)) {
                        option {
                            key: "{format.id}",
                            value: format.id,
                            selected: shown == Some(format.id),
                            disabled: !download && !format.supported(),
                            {option_text(format.label, format.supported(), use_for, current == Some(format.id))}
                        }
                    }
                }
            }
            if download {
                option { value: OTHER_FORMAT, selected: value == OTHER_FORMAT, "Other…" }
            }
        }
    }
}

fn option_text(label: &str, supported: bool, use_for: FormatUse, current: bool) -> String {
    match (use_for, supported) {
        (FormatUse::Download, false) => format!("{label} (download only)"),
        (FormatUse::Write, false) => format!("{label} (not supported yet)"),
        _ if current => format!("{label} (current)"),
        _ => label.to_string(),
    }
}
