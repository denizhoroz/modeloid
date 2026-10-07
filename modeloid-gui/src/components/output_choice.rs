use dioxus::prelude::*;

use crate::api::datasets::{validate_dataset_name, OutputTarget};
use crate::components::{IconName, TitleIcon};
use crate::components::InfoTip;

/// Where an `OutputSelect` sends the result.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputMode {
    #[default]
    New,
    Overwrite,
    /// A dataset link: a new dataset sharing the source's images (Classes & Merge).
    Link,
}

/// What the user chose in an `OutputSelect`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OutputChoice {
    pub mode: OutputMode,
    /// Typed name; `None` until the user edits it, so the suggestion is used.
    pub name: Option<String>,
    /// Ticked "replace the source" confirmation.
    pub confirmed: bool,
}

impl OutputChoice {
    /// The name that will be used: the typed one or `suggested`.
    pub fn name_or<'a>(&'a self, suggested: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(suggested)
    }

    pub fn resolve(&self, suggested: &str, can_overwrite: bool) -> Result<OutputTarget, String> {
        // Overwrite falls back to "new" when it isn't possible (the form shows it that way).
        if self.mode == OutputMode::Overwrite && can_overwrite {
            if !self.confirmed {
                return Err("Confirm that the source dataset will be replaced.".into());
            }
            return Ok(OutputTarget::Overwrite);
        }
        let name = self.name_or(suggested).to_string();
        validate_dataset_name(&name)?;
        Ok(if self.mode == OutputMode::Link { OutputTarget::Link { name } } else { OutputTarget::New { name } })
    }
}

/// Reusable "where does the result go" section: a new dataset (name prefilled
/// from `suggested_name`), a dataset link (with `allow_link`) or overwrite the
/// source, which needs a confirmation.
#[component]
pub fn OutputSelect(
    choice: Signal<OutputChoice>,
    /// Name for the new dataset until the user types one, e.g. "cats-split".
    #[props(into)]
    suggested_name: String,
    /// Name of the dataset that would be replaced; `None` hides "Overwrite"
    /// (no input yet, or several inputs).
    #[props(default)]
    source_name: Option<String>,
    /// Show validation messages (after a submit attempt).
    #[props(default)]
    show_errors: bool,
    #[props(default)] disabled: bool,
    /// Offer "Create dataset link" (a new dataset sharing the source's images).
    #[props(default)]
    allow_link: bool,
    /// Prefix of the element ids; give each `OutputSelect` on a page its own.
    #[props(into, default = "output".to_string())]
    id: String,
) -> Element {
    let mut choice = choice;
    let can_overwrite = source_name.is_some();
    let mode = choice.read().mode;
    let overwrite = mode == OutputMode::Overwrite && can_overwrite;
    let link = mode == OutputMode::Link && allow_link;
    let new = !overwrite && !link;
    let name = choice.read().name_or(&suggested_name).to_string();

    let edited = choice.read().name.is_some();
    let error = if show_errors || edited { choice.read().resolve(&suggested_name, can_overwrite).err() } else { None };

    rsx! {
        fieldset { class: "section-card", disabled,
            legend { class: "section-card-title", TitleIcon { name: IconName::Save } "Output" }

            div { class: "choice-list",
                label { class: if new { "choice is-selected" } else { "choice" },
                    input {
                        r#type: "radio",
                        name: "{id}-select",
                        checked: new,
                        onchange: move |_| choice.write().mode = OutputMode::New,
                    }
                    span { class: "choice-body",
                        span { class: "choice-label",
                            "Save as new dataset "
                            InfoTip { label: "About saving as new", text: "The source stays as it is." }
                        }
                    }
                }

                if allow_link {
                    label { class: if link { "choice is-selected" } else { "choice" },
                        input {
                            r#type: "radio",
                            name: "{id}-select",
                            checked: link,
                            onchange: move |_| choice.write().mode = OutputMode::Link,
                        }
                        span { class: "choice-body",
                            span { class: "choice-label",
                                "Create dataset link "
                                InfoTip {
                                    label: "About dataset links",
                                    text: "A new dataset that shares the source's image files instead of copying them (hard links: no extra storage), with its own labels and class names. Images dropped here are left out. Deleting or changing the source later doesn't affect it. Needs a drive that supports hard links.",
                                }
                            }
                        }
                    }
                }

                label { class: if overwrite { "choice is-selected" } else if can_overwrite { "choice" } else { "choice is-disabled" },
                    input {
                        r#type: "radio",
                        name: "{id}-select",
                        checked: overwrite,
                        disabled: !can_overwrite,
                        onchange: move |_| choice.write().mode = OutputMode::Overwrite,
                    }
                    span { class: "choice-body",
                        span { class: "choice-label", "Overwrite source" }
                        match &source_name {
                            Some(source) => rsx! { span { class: "choice-detail", title: "{source}", "Replaces {source}." } },
                            None => rsx! { span { class: "choice-detail is-empty", "Only with a single input dataset." } },
                        }
                    }
                }
            }

            if overwrite {
                label { class: "check",
                    input {
                        r#type: "checkbox",
                        checked: choice.read().confirmed,
                        onchange: move |e| choice.write().confirmed = e.checked(),
                    }
                    span { "I understand "
                        strong { "{source_name.clone().unwrap_or_default()}" }
                        " will be replaced. This can't be undone."
                    }
                }
            } else {
                div { class: "field",
                    label { class: "field-label", r#for: "{id}-name",
                        "Dataset name "
                        InfoTip { label: "About the name", text: "Folder name inside the workspace's datasets folder." }
                    }
                    input {
                        id: "{id}-name",
                        class: "input",
                        r#type: "text",
                        spellcheck: false,
                        autocomplete: "off",
                        value: "{name}",
                        aria_invalid: if error.is_some() { "true" },
                        oninput: move |e| choice.write().name = Some(e.value()),
                    }
                }
            }

            if let Some(message) = error {
                p { class: "field-error", role: "alert", "{message}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_choices() {
        let new = OutputChoice::default();
        assert_eq!(new.resolve("cats-split", true), Ok(OutputTarget::New { name: "cats-split".into() }));

        let typed = OutputChoice { name: Some("mine".into()), ..Default::default() };
        assert_eq!(typed.resolve("cats-split", true), Ok(OutputTarget::New { name: "mine".into() }));

        let bad = OutputChoice { name: Some("a/b".into()), ..Default::default() };
        assert!(bad.resolve("x", true).is_err());

        let unconfirmed = OutputChoice { mode: OutputMode::Overwrite, ..Default::default() };
        assert!(unconfirmed.resolve("x", true).is_err());

        let link = OutputChoice { mode: OutputMode::Link, ..Default::default() };
        assert_eq!(link.resolve("cats-classes", true), Ok(OutputTarget::Link { name: "cats-classes".into() }));
        let bad_link = OutputChoice { mode: OutputMode::Link, name: Some("a/b".into()), ..Default::default() };
        assert!(bad_link.resolve("x", true).is_err());

        let confirmed = OutputChoice { mode: OutputMode::Overwrite, confirmed: true, ..Default::default() };
        assert_eq!(confirmed.resolve("x", true), Ok(OutputTarget::Overwrite));
        assert_eq!(confirmed.resolve("x", false), Ok(OutputTarget::New { name: "x".into() }));
    }
}
