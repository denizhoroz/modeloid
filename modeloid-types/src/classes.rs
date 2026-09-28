//! Classes & Merge: map the classes of one or more datasets onto an ordered
//! target class list, then write one dataset. With one input this edits
//! classes (rename, merge, drop, reorder); with several it merges datasets.

use serde::{Deserialize, Serialize};

/// A saved target class list. Its order sets the class ids (YOLO).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassList {
    pub name: String,
    pub classes: Vec<String>,
}

/// What dropping a class removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DropMode {
    /// Only its boxes; the objects stay in the images, unlabeled.
    Boxes,
    /// Every image that contains it.
    Images,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassAction {
    /// Becomes this target class.
    To(String),
    Drop(DropMode),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitMode {
    /// Everything goes into train; re-split later with Split & Balance.
    #[default]
    AllTrain,
    /// Images keep the split they had in their dataset.
    Keep,
}

/// Rule for a source class name, in every input that has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassRule {
    pub source: String,
    pub action: ClassAction,
}

/// Rule for a source class in one input only; wins over `ClassRule`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassOverride {
    pub dataset: u32,
    pub source: String,
    pub action: ClassAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePlan {
    /// Input dataset ids, in order.
    pub inputs: Vec<u32>,
    /// Output classes, in id order.
    pub targets: Vec<String>,
    pub rules: Vec<ClassRule>,
    #[serde(default)]
    pub overrides: Vec<ClassOverride>,
    #[serde(default)]
    pub split: SplitMode,
    /// Output label format: "yolo", "coco" or "voc".
    pub format: String,
}

impl MergePlan {
    /// What happens to class `class` of input `dataset`: its override, else
    /// the rule for its name, else nothing (unmapped).
    pub fn resolve(&self, dataset: u32, class: &str) -> Option<&ClassAction> {
        self.overrides
            .iter()
            .find(|o| o.dataset == dataset && o.source == class)
            .map(|o| &o.action)
            .or_else(|| self.rules.iter().find(|r| r.source == class).map(|r| &r.action))
    }

    /// Sets the rule for source name `source` (replacing an existing one).
    pub fn set_rule(&mut self, source: &str, action: ClassAction) {
        match self.rules.iter_mut().find(|r| r.source == source) {
            Some(rule) => rule.action = action,
            None => self.rules.push(ClassRule { source: source.to_string(), action }),
        }
    }

    /// Sets (`Some`) or removes (`None`) the override for one input's class.
    pub fn set_override(&mut self, dataset: u32, source: &str, action: Option<ClassAction>) {
        self.overrides.retain(|o| !(o.dataset == dataset && o.source == source));
        if let Some(action) = action {
            self.overrides.push(ClassOverride { dataset, source: source.to_string(), action });
        }
    }

    /// Renames target `from` to `to`, updating every rule that points at it.
    pub fn rename_target(&mut self, from: &str, to: &str) {
        for t in self.targets.iter_mut().filter(|t| *t == from) {
            *t = to.to_string();
        }
        let actions = self.rules.iter_mut().map(|r| &mut r.action).chain(self.overrides.iter_mut().map(|o| &mut o.action));
        for action in actions {
            if matches!(action, ClassAction::To(t) if t == from) {
                *action = ClassAction::To(to.to_string());
            }
        }
    }

    /// Removes target `name`; rules and overrides pointing at it are removed
    /// too, so those classes show as unmapped until given a new rule.
    pub fn remove_target(&mut self, name: &str) {
        self.targets.retain(|t| t != name);
        let points_here = |a: &ClassAction| matches!(a, ClassAction::To(t) if t == name);
        self.rules.retain(|r| !points_here(&r.action));
        self.overrides.retain(|o| !points_here(&o.action));
    }

    /// Problems that stop the plan from running, as messages.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.inputs.is_empty() {
            problems.push("Choose at least one dataset.".into());
        }
        if self.targets.is_empty() {
            problems.push("Add at least one target class.".into());
        }
        for (i, t) in self.targets.iter().enumerate() {
            if t.trim().is_empty() {
                problems.push(format!("Target class {} has no name.", i + 1));
            } else if self.targets[..i].contains(t) {
                problems.push(format!("Target class \"{t}\" appears twice."));
            }
        }
        let actions = self.rules.iter().map(|r| &r.action).chain(self.overrides.iter().map(|o| &o.action));
        for action in actions {
            if let ClassAction::To(t) = action {
                if !self.targets.contains(t) {
                    problems.push(format!("\"{t}\" isn't in the target classes."));
                }
            }
        }
        problems.dedup();
        problems
    }
}

/// Key for matching class names that differ only in case, separators or a
/// plural "s": `Traffic_Lights` and `traffic-light` both give `traffic light`.
pub fn normalize_class(name: &str) -> String {
    let words: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c == '_' || c == '-' || c.is_whitespace() { ' ' } else { c })
        .collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    match words.strip_suffix('s') {
        Some(stem) if stem.len() >= 3 && !stem.ends_with('s') => stem.to_string(),
        _ => words,
    }
}

/// A source class across the inputs: labels and images per input (plan's input order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceClass {
    pub name: String,
    pub labels: Vec<u32>,
    pub images: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCount {
    pub target: String,
    /// Labels per input.
    pub labels: Vec<u32>,
}

/// A target class labelled in some inputs but not others: images from the
/// others may contain unlabeled objects of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingLabels {
    pub target: String,
    pub labelled_in: Vec<u32>,
    pub missing_in: Vec<u32>,
    /// Images in `missing_in` inputs.
    pub images_at_risk: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreview {
    /// Input names, in plan order.
    pub input_names: Vec<String>,
    /// Images per input.
    pub input_images: Vec<u32>,
    pub sources: Vec<SourceClass>,
    pub targets: Vec<TargetCount>,
    /// (dataset id, class) with labels but no rule.
    pub unmapped: Vec<(u32, String)>,
    pub empty_targets: Vec<String>,
    pub missing_labels: Vec<MissingLabels>,
    /// Labels removed by "drop boxes" rules.
    pub dropped_labels: u32,
    /// Images containing a "drop images" class (an upper bound: an image
    /// with two such classes counts twice).
    pub dropped_images: u32,
    /// Plan problems (see `MergePlan::problems`).
    pub problems: Vec<String>,
}

impl MergePreview {
    pub fn can_run(&self) -> bool {
        self.unmapped.is_empty() && self.problems.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize_class("Traffic_Lights"), "traffic light");
        assert_eq!(normalize_class(" traffic-light "), "traffic light");
        assert_eq!(normalize_class("cars"), "car");
        assert_eq!(normalize_class("bus"), "bus", "short words keep their s");
        assert_eq!(normalize_class("glass"), "glass", "double s isn't a plural");
    }

    #[test]
    fn overrides_win_and_renames_follow() {
        let mut plan = MergePlan { inputs: vec![1, 2], targets: vec!["car".into()], format: "yolo".into(), ..Default::default() };
        plan.set_rule("vehicle", ClassAction::To("car".into()));
        plan.set_override(2, "vehicle", Some(ClassAction::Drop(DropMode::Images)));
        assert_eq!(plan.resolve(1, "vehicle"), Some(&ClassAction::To("car".into())));
        assert_eq!(plan.resolve(2, "vehicle"), Some(&ClassAction::Drop(DropMode::Images)));
        assert_eq!(plan.resolve(1, "person"), None);

        plan.rename_target("car", "automobile");
        assert_eq!(plan.targets, ["automobile"]);
        assert_eq!(plan.resolve(1, "vehicle"), Some(&ClassAction::To("automobile".into())));
        assert!(plan.problems().is_empty());

        plan.set_rule("person", ClassAction::To("human".into()));
        plan.targets.push("automobile".into());
        let problems = plan.problems();
        assert!(problems.iter().any(|p| p.contains("twice")) && problems.iter().any(|p| p.contains("human")), "{problems:?}");
    }
}
