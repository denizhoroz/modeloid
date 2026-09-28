//! Classes & Merge: suggest a class mapping, preview its result from cached
//! statistics, and apply it to label sets (one input: class edit; several: merge).

use std::collections::HashMap;

use crate::core::{
    labels::{LabelSet, Split},
    stats::ExploreStats,
};

pub use modeloid_types::classes::*;

/// One input for a suggestion or preview: id, name, statistics.
pub struct InputStats<'a> {
    pub id: u32,
    pub name: &'a str,
    pub stats: &'a ExploreStats,
}

/// A first mapping: classes whose names match after `normalize_class` join
/// into one target (spelled as first seen). Targets follow the first input's
/// class order, then new names as they appear. Classes without labels are left out.
pub fn suggest(inputs: &[InputStats], format: &str) -> MergePlan {
    let mut plan = MergePlan { inputs: inputs.iter().map(|i| i.id).collect(), format: format.to_string(), ..Default::default() };
    let mut target_of: HashMap<String, String> = HashMap::new();
    for input in inputs {
        for class in input.stats.classes.iter().filter(|c| c.total_labels() > 0) {
            let key = normalize_class(&class.name);
            let target = target_of
                .entry(key)
                .or_insert_with(|| {
                    plan.targets.push(class.name.clone());
                    class.name.clone()
                })
                .clone();
            plan.set_rule(&class.name, ClassAction::To(target));
        }
    }
    plan
}

/// The plan's result, counted from the inputs' statistics (no label reading).
pub fn preview(plan: &MergePlan, inputs: &[InputStats]) -> MergePreview {
    let n = inputs.len();
    let mut preview = MergePreview {
        input_names: inputs.iter().map(|i| i.name.to_string()).collect(),
        input_images: inputs.iter().map(|i| i.stats.image_count).collect(),
        targets: plan.targets.iter().map(|t| TargetCount { target: t.clone(), labels: vec![0; n] }).collect(),
        problems: plan.problems(),
        ..Default::default()
    };

    for (slot, input) in inputs.iter().enumerate() {
        for class in &input.stats.classes {
            let (labels, images) = (class.total_labels(), class.images);
            let source = match preview.sources.iter_mut().find(|s| s.name == class.name) {
                Some(source) => source,
                None => {
                    preview.sources.push(SourceClass { name: class.name.clone(), labels: vec![0; n], images: vec![0; n] });
                    preview.sources.last_mut().unwrap()
                }
            };
            source.labels[slot] += labels;
            source.images[slot] += images;
            if labels == 0 {
                continue;
            }
            match plan.resolve(input.id, &class.name) {
                Some(ClassAction::To(target)) => {
                    if let Some(t) = preview.targets.iter_mut().find(|t| t.target == *target) {
                        t.labels[slot] += labels;
                    }
                }
                Some(ClassAction::Drop(DropMode::Boxes)) => preview.dropped_labels += labels,
                Some(ClassAction::Drop(DropMode::Images)) => preview.dropped_images += images,
                None => preview.unmapped.push((input.id, class.name.clone())),
            }
        }
    }
    // Rows without labels anywhere are noise (e.g. numbered YOLO classes).
    preview.sources.retain(|s| s.labels.iter().any(|&l| l > 0));

    for target in &preview.targets {
        let labelled: Vec<usize> = (0..n).filter(|&i| target.labels[i] > 0).collect();
        if labelled.is_empty() {
            preview.empty_targets.push(target.target.clone());
        } else if n > 1 && labelled.len() < n {
            let missing: Vec<usize> = (0..n).filter(|i| !labelled.contains(i)).collect();
            preview.missing_labels.push(MissingLabels {
                target: target.target.clone(),
                labelled_in: labelled.iter().map(|&i| inputs[i].id).collect(),
                images_at_risk: missing.iter().map(|&i| inputs[i].stats.image_count).sum(),
                missing_in: missing.iter().map(|&i| inputs[i].id).collect(),
            });
        }
    }
    preview
}

/// What `apply` did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeSummary {
    pub images: usize,
    pub removed_images: usize,
    pub dropped_objects: usize,
}

/// Maps and joins `inputs` (id, labels) into one label set with the plan's
/// target classes. Fails on an object whose class has no rule.
pub fn apply(plan: &MergePlan, inputs: Vec<(u32, LabelSet)>) -> Result<(LabelSet, MergeSummary), String> {
    if let Some(problem) = plan.problems().into_iter().next() {
        return Err(problem);
    }
    let mut out = LabelSet { classes: plan.targets.clone(), declared_classes: Some(plan.targets.len()), ..Default::default() };
    let mut summary = MergeSummary::default();

    for (id, set) in inputs {
        // Per source class id: the action, resolved once.
        let actions: Vec<Option<&ClassAction>> = set.classes.iter().map(|c| plan.resolve(id, c)).collect();
        let target_index = |name: &str| plan.targets.iter().position(|t| t == name);
        out.warnings.extend(set.warnings);

        for mut image in set.images {
            let mut drop_image = false;
            let mut objects = Vec::with_capacity(image.objects.len());
            for mut object in image.objects {
                match actions.get(object.class).copied().flatten() {
                    Some(ClassAction::To(target)) => {
                        object.class = target_index(target).ok_or_else(|| format!("\"{target}\" isn't a target class"))?;
                        objects.push(object);
                    }
                    Some(ClassAction::Drop(DropMode::Boxes)) => summary.dropped_objects += 1,
                    Some(ClassAction::Drop(DropMode::Images)) => drop_image = true,
                    None => {
                        let name = set.classes.get(object.class).cloned().unwrap_or_else(|| object.class.to_string());
                        return Err(format!("class \"{name}\" of dataset #{id} has no rule"));
                    }
                }
            }
            if drop_image {
                summary.removed_images += 1;
                continue;
            }
            image.objects = objects;
            if plan.split == SplitMode::AllTrain {
                image.split = Some(Split::Train);
            }
            out.images.push(image);
        }
    }
    summary.images = out.images.len();
    Ok((out, summary))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::labels::{BBox, ImageLabels, Object};
    use crate::core::stats::analyze;

    fn set(classes: &[&str], images: Vec<(Option<Split>, Vec<usize>)>) -> LabelSet {
        LabelSet {
            classes: classes.iter().map(|c| c.to_string()).collect(),
            images: images
                .into_iter()
                .enumerate()
                .map(|(i, (split, objects))| ImageLabels {
                    file: PathBuf::from(format!("{i}.jpg")),
                    name: format!("{i}.jpg"),
                    split,
                    width: 100,
                    height: 100,
                    objects: objects
                        .into_iter()
                        .map(|class| Object { class, bbox: BBox { x: 10.0, y: 10.0, w: 20.0, h: 20.0 }, polygon: None })
                        .collect(),
                })
                .collect(),
            ..Default::default()
        }
    }

    /// A: car ×2, person ×1 (val), bike ×1. B: Cars ×1, dog ×1 + car in the same image.
    fn inputs() -> (LabelSet, LabelSet) {
        let a = set(&["car", "person", "bike"], vec![(Some(Split::Train), vec![0, 0]), (Some(Split::Val), vec![1]), (Some(Split::Train), vec![2])]);
        let b = set(&["Cars", "dog"], vec![(Some(Split::Test), vec![0, 1]), (None, vec![])]);
        (a, b)
    }

    #[test]
    fn suggests_joins_by_normalized_name() {
        let (a, b) = inputs();
        let (sa, sb) = (analyze(&a), analyze(&b));
        let plan = suggest(&[InputStats { id: 1, name: "a", stats: &sa }, InputStats { id: 2, name: "b", stats: &sb }], "yolo");
        assert_eq!(plan.targets, ["car", "person", "bike", "dog"]);
        assert_eq!(plan.resolve(2, "Cars"), Some(&ClassAction::To("car".into())));
    }

    #[test]
    fn previews_counts_and_missing_labels() {
        let (a, b) = inputs();
        let (sa, sb) = (analyze(&a), analyze(&b));
        let inputs = [InputStats { id: 1, name: "a", stats: &sa }, InputStats { id: 2, name: "b", stats: &sb }];
        let mut plan = suggest(&inputs, "yolo");
        plan.set_rule("bike", ClassAction::Drop(DropMode::Boxes));
        plan.remove_target("bike");
        plan.targets.push("truck".into());

        let p = preview(&plan, &inputs);
        let car = p.targets.iter().find(|t| t.target == "car").unwrap();
        assert_eq!(car.labels, [2, 1]);
        assert_eq!(p.dropped_labels, 1);
        assert_eq!(p.empty_targets, ["truck"]);
        let person = p.missing_labels.iter().find(|m| m.target == "person").unwrap();
        assert_eq!((person.missing_in.as_slice(), person.images_at_risk), (&[2u32][..], 2));
        assert!(p.can_run());

        plan.rules.retain(|r| r.source != "dog");
        let p = preview(&plan, &inputs);
        assert_eq!(p.unmapped, [(2, "dog".to_string())]);
        assert!(!p.can_run());
    }

    #[test]
    fn applies_mapping_drops_and_split_mode() {
        let (a, b) = inputs();
        let (sa, sb) = (analyze(&a), analyze(&b));
        let mut plan = suggest(&[InputStats { id: 1, name: "a", stats: &sa }, InputStats { id: 2, name: "b", stats: &sb }], "yolo");
        plan.set_rule("bike", ClassAction::Drop(DropMode::Boxes));
        plan.remove_target("bike");
        plan.remove_target("dog");
        plan.set_rule("dog", ClassAction::Drop(DropMode::Boxes));
        // Dataset 2's dogs take the whole image with them.
        plan.set_override(2, "dog", Some(ClassAction::Drop(DropMode::Images)));

        let (out, summary) = apply(&plan, vec![(1, a.clone()), (2, b.clone())]).unwrap();
        assert_eq!(out.classes, ["car", "person"]);
        assert_eq!(out.declared_classes, Some(2));
        assert_eq!(summary, MergeSummary { images: 4, removed_images: 1, dropped_objects: 1 });
        assert!(out.images.iter().all(|i| i.split == Some(Split::Train)), "all into train by default");
        assert_eq!(out.images[1].objects[0].class, 1, "person re-indexed");

        plan.split = SplitMode::Keep;
        let (out, _) = apply(&plan, vec![(1, a.clone()), (2, b.clone())]).unwrap();
        assert_eq!(out.images[1].split, Some(Split::Val));
        assert_eq!(out.images[3].split, None, "b's unsplit image keeps no split");

        plan.rules.retain(|r| r.source != "person");
        assert!(apply(&plan, vec![(1, a), (2, b)]).unwrap_err().contains("person"));
    }
}
