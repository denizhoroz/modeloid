//! Plan edits made by the Classes & Merge page, kept free of UI so they can
//! be tested.

use modeloid_types::classes::{normalize_class, ClassAction, ClassList, DropMode, MergePlan};
use modeloid_types::colors;

/// What a mapping `select` shows for a class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Target(String),
    /// Make a new target named after the source class.
    New,
    DropBoxes,
    DropImages,
    Unmapped,
}

impl Choice {
    pub fn of(action: Option<&ClassAction>) -> Choice {
        match action {
            Some(ClassAction::To(t)) => Choice::Target(t.clone()),
            Some(ClassAction::Drop(DropMode::Boxes)) => Choice::DropBoxes,
            Some(ClassAction::Drop(DropMode::Images)) => Choice::DropImages,
            None => Choice::Unmapped,
        }
    }

    /// `select` option value.
    pub fn value(&self) -> String {
        match self {
            Choice::Target(t) => format!("t:{t}"),
            Choice::New => "new".into(),
            Choice::DropBoxes => "drop:boxes".into(),
            Choice::DropImages => "drop:images".into(),
            Choice::Unmapped => String::new(),
        }
    }

    pub fn parse(value: &str) -> Choice {
        match value {
            "new" => Choice::New,
            "drop:boxes" => Choice::DropBoxes,
            "drop:images" => Choice::DropImages,
            v => v.strip_prefix("t:").map_or(Choice::Unmapped, |t| Choice::Target(t.to_string())),
        }
    }

    /// The action for `source`, adding a target to `plan` for `New`.
    /// `None` = unmapped.
    pub fn into_action(self, plan: &mut MergePlan, source: &str) -> Option<ClassAction> {
        match self {
            Choice::Target(t) => Some(ClassAction::To(t)),
            Choice::New => {
                if !plan.targets.iter().any(|t| t == source) {
                    plan.targets.push(source.to_string());
                }
                Some(ClassAction::To(source.to_string()))
            }
            Choice::DropBoxes => Some(ClassAction::Drop(DropMode::Boxes)),
            Choice::DropImages => Some(ClassAction::Drop(DropMode::Images)),
            Choice::Unmapped => None,
        }
    }
}

/// Sets what happens to `source` in every input.
pub fn set_choice(plan: &mut MergePlan, source: &str, choice: Choice) {
    match choice.into_action(plan, source) {
        Some(action) => plan.set_rule(source, action),
        None => plan.rules.retain(|r| r.source != source),
    }
}

/// Sets (or with `None`, clears) one input's override for `source`.
pub fn set_override_choice(plan: &mut MergePlan, dataset: u32, source: &str, choice: Option<Choice>) {
    let action = choice.and_then(|c| c.into_action(plan, source));
    plan.set_override(dataset, source, action);
}

/// Renames a target; `Err` with a message if the new name is empty or taken.
pub fn rename_target(plan: &mut MergePlan, from: &str, to: &str) -> Result<(), String> {
    let to = to.trim();
    if to.is_empty() {
        return Err("A class needs a name.".into());
    }
    if to != from && plan.targets.iter().any(|t| t == to) {
        return Err(format!("\"{to}\" is already a target class. Map classes to it instead."));
    }
    plan.rename_target(from, to);
    Ok(())
}

pub fn add_target(plan: &mut MergePlan, name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Type a class name.".into());
    }
    if plan.targets.iter().any(|t| t == name) {
        return Err(format!("\"{name}\" is already there."));
    }
    plan.targets.push(name.to_string());
    Ok(())
}

/// Sets target `name`'s colour; `Err` if `color` isn't `#rrggbb`.
pub fn set_color(plan: &mut MergePlan, name: &str, color: &str) -> Result<(), String> {
    let color = colors::normalize(color).ok_or_else(|| format!("\"{color}\" isn't a colour."))?;
    plan.colors.insert(name.to_string(), color);
    Ok(())
}

pub fn move_target(plan: &mut MergePlan, index: usize, up: bool) {
    let other = if up { index.checked_sub(1) } else { Some(index + 1) };
    if let Some(other) = other.filter(|&o| o < plan.targets.len()) {
        plan.targets.swap(index, other);
    }
}

/// Uses `list` as the target classes. Rules pointing at classes not in the
/// list are dropped; then `sources` without a rule are matched to list
/// entries with the same normalized name.
pub fn apply_class_list(plan: &mut MergePlan, list: &ClassList, sources: &[String]) {
    for old in plan.targets.clone() {
        if !list.classes.contains(&old) {
            plan.remove_target(&old);
        }
    }
    plan.targets = list.classes.clone();
    for source in sources {
        if plan.rules.iter().any(|r| r.source == *source) {
            continue;
        }
        let key = normalize_class(source);
        if let Some(target) = list.classes.iter().find(|c| normalize_class(c) == key) {
            plan.set_rule(source, ClassAction::To(target.clone()));
        }
    }
}

/// A fresh suggestion for new inputs, keeping the user's edits where they
/// still apply: earlier targets (and order) first, earlier rules for class
/// names still present, overrides for inputs still chosen, format and splits.
pub fn carry_over(old: &MergePlan, mut new: MergePlan) -> MergePlan {
    let suggested_targets = std::mem::take(&mut new.targets);
    let mut targets = old.targets.clone();
    for rule in &old.rules {
        if new.rules.iter().any(|r| r.source == rule.source) {
            new.set_rule(&rule.source, rule.action.clone());
        }
    }
    // Suggested targets that something still maps to.
    for t in suggested_targets {
        let used = new.rules.iter().any(|r| r.action == ClassAction::To(t.clone()));
        if used && !targets.contains(&t) {
            targets.push(t);
        }
    }
    new.targets = targets;
    new.overrides = old.overrides.iter().filter(|o| new.inputs.contains(&o.dataset)).cloned().collect();
    new.split = old.split;
    if !old.format.is_empty() && !old.inputs.is_empty() {
        new.format = old.format.clone();
    }
    // Colours the user had keep winning over the inputs'.
    new.colors.extend(old.colors.iter().map(|(k, v)| (k.clone(), v.clone())));
    new.fill_colors();
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> MergePlan {
        let mut plan = MergePlan { inputs: vec![1], targets: vec!["car".into(), "person".into()], format: "yolo".into(), ..Default::default() };
        plan.set_rule("car", ClassAction::To("car".into()));
        plan.set_rule("Cars", ClassAction::To("car".into()));
        plan.set_rule("person", ClassAction::To("person".into()));
        plan
    }

    #[test]
    fn choices_roundtrip_and_apply() {
        for choice in [Choice::Target("a b".into()), Choice::New, Choice::DropBoxes, Choice::DropImages, Choice::Unmapped] {
            assert_eq!(Choice::parse(&choice.value()), choice);
        }
        let mut p = plan();
        set_choice(&mut p, "bike", Choice::New);
        assert_eq!(p.targets, ["car", "person", "bike"]);
        set_choice(&mut p, "person", Choice::DropImages);
        assert_eq!(p.resolve(1, "person"), Some(&ClassAction::Drop(DropMode::Images)));
        set_choice(&mut p, "person", Choice::Unmapped);
        assert_eq!(p.resolve(1, "person"), None);
        set_override_choice(&mut p, 1, "car", Some(Choice::DropBoxes));
        assert_eq!(p.resolve(1, "car"), Some(&ClassAction::Drop(DropMode::Boxes)));
        set_override_choice(&mut p, 1, "car", None);
        assert_eq!(p.resolve(1, "car"), Some(&ClassAction::To("car".into())));
    }

    #[test]
    fn edits_targets() {
        let mut p = plan();
        assert!(rename_target(&mut p, "car", "person").is_err());
        assert!(rename_target(&mut p, "car", " ").is_err());
        rename_target(&mut p, "car", "vehicle").unwrap();
        assert_eq!(p.resolve(1, "Cars"), Some(&ClassAction::To("vehicle".into())));
        add_target(&mut p, "dog").unwrap();
        assert!(add_target(&mut p, "dog").is_err());
        move_target(&mut p, 2, true);
        assert_eq!(p.targets, ["vehicle", "dog", "person"]);
        move_target(&mut p, 0, true);
        assert_eq!(p.targets[0], "vehicle", "first can't move up");
    }

    #[test]
    fn applies_class_lists() {
        let mut p = plan();
        let list = ClassList { name: "std".into(), classes: vec!["person".into(), "Car".into()] };
        apply_class_list(&mut p, &list, &["car".into(), "Cars".into(), "person".into()]);
        assert_eq!(p.targets, ["person", "Car"]);
        assert_eq!(p.resolve(1, "person"), Some(&ClassAction::To("person".into())));
        assert_eq!(p.resolve(1, "Cars"), Some(&ClassAction::To("Car".into())), "rematched by normalized name");
    }

    #[test]
    fn carries_edits_over_to_new_inputs() {
        let mut old = plan();
        old.split = modeloid_types::classes::SplitMode::Keep;
        set_choice(&mut old, "person", Choice::DropBoxes);
        let mut new = MergePlan { inputs: vec![1, 2], targets: vec!["car".into(), "person".into(), "dog".into()], format: "coco".into(), ..Default::default() };
        for (s, t) in [("car", "car"), ("person", "person"), ("dog", "dog")] {
            new.set_rule(s, ClassAction::To(t.into()));
        }
        let merged = carry_over(&old, new);
        assert_eq!(merged.targets, ["car", "person", "dog"]);
        assert_eq!(merged.resolve(2, "person"), Some(&ClassAction::Drop(DropMode::Boxes)), "edit kept");
        assert_eq!(merged.format, "yolo");
        assert_eq!(merged.split, modeloid_types::classes::SplitMode::Keep);
    }

    #[test]
    fn renamed_class_keeps_its_labels_after_an_overwrite() {
        // Before the run: cat renamed to kitty.
        let mut old = MergePlan { inputs: vec![1], targets: vec!["cat".into(), "dog".into()], format: "yolo".into(), ..Default::default() };
        old.set_rule("cat", ClassAction::To("cat".into()));
        old.set_rule("dog", ClassAction::To("dog".into()));
        rename_target(&mut old, "cat", "kitty").unwrap();
        // After overwriting, the dataset's class is called kitty.
        let mut new = MergePlan { inputs: vec![1], targets: vec!["kitty".into(), "dog".into()], format: "yolo".into(), ..Default::default() };
        new.set_rule("kitty", ClassAction::To("kitty".into()));
        new.set_rule("dog", ClassAction::To("dog".into()));

        let merged = carry_over(&old, new);
        assert_eq!(merged.targets, ["kitty", "dog"]);
        assert_eq!(merged.resolve(1, "kitty"), Some(&ClassAction::To("kitty".into())));
        assert!(merged.rules.iter().all(|r| r.source != "cat"));
    }

    #[test]
    fn targets_keep_their_colours() {
        let mut plan = plan();
        plan.fill_colors();
        assert_eq!(plan.colors.len(), 2);
        set_color(&mut plan, "car", "#FF0000").unwrap();
        assert!(set_color(&mut plan, "car", "red").is_err());
        rename_target(&mut plan, "car", "auto").unwrap();
        assert_eq!(plan.colors.get("auto").map(String::as_str), Some("#ff0000"), "renaming keeps the colour");
        add_target(&mut plan, "bus").unwrap();
        plan.fill_colors();
        assert!(plan.colors.contains_key("bus"));

        // A fresh suggestion keeps the colours the user chose.
        let new = MergePlan { inputs: vec![1], targets: vec!["auto".into()], colors: [("auto".to_string(), "#00ff00".to_string())].into(), format: "yolo".into(), ..Default::default() };
        assert_eq!(carry_over(&plan, new).colors.get("auto").map(String::as_str), Some("#ff0000"));
    }
}
