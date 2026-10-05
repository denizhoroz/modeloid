//! Split & Balance: choose which images of a label set stay (class filter,
//! random subset, per-class cap) and which split each one goes to. The same
//! run gives the preview's counts and the applied result.

use crate::core::{
    labels::{ImageLabels, LabelSet, Split},
    stats::{PerSplit, SPLITS, split_slot},
};

pub use modeloid_types::balance::*;

/// Small seeded generator (SplitMix64), so a seed gives the same result on
/// every machine, whatever the versions of the dependencies.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        ((u128::from(self.next()) * n as u128) >> 64) as usize
    }

    pub(crate) fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.below(i + 1));
        }
    }
}

/// Class ids of the image's objects, without those of left-out classes
/// (`dropped`, by class id).
fn classes_of<'a>(image: &'a ImageLabels, dropped: &'a [bool]) -> impl Iterator<Item = usize> + 'a {
    image.objects.iter().map(|o| o.class).filter(|&class| dropped.get(class) != Some(&true))
}

/// Labels per class in `images` (positions in `set.images`).
fn class_totals(set: &LabelSet, images: &[usize], dropped: &[bool]) -> Vec<u32> {
    let mut totals = vec![0; set.classes.len()];
    for &i in images {
        for class in classes_of(&set.images[i], dropped) {
            if let Some(total) = totals.get_mut(class) {
                *total += 1;
            }
        }
    }
    totals
}

/// The image's rarest class as (its labels in `totals`, class id); `None`
/// for an image without objects of a known class.
fn rarest_class(image: &ImageLabels, totals: &[u32], dropped: &[bool]) -> Option<(u32, usize)> {
    classes_of(image, dropped).filter_map(|class| totals.get(class).map(|&total| (total, class))).min()
}

/// Splits for `count` images in a row, in `shares` percent (train, val,
/// test): each image goes to the split furthest behind its share, so every
/// run of images, and the whole, gets the shares to within one image.
fn assign(count: usize, shares: [u32; 3]) -> Vec<Split> {
    const ORDER: [Split; 3] = [Split::Train, Split::Val, Split::Test];
    let mut given = [0i64; 3];
    (1..=count as i64)
        .map(|n| {
            // Train first on a tie.
            let slot = (0..3).max_by_key(|&s| (i64::from(shares[s]) * n - 100 * given[s], std::cmp::Reverse(s))).unwrap_or(0);
            given[slot] += 1;
            ORDER[slot]
        })
        .collect()
}

/// Images and labels per class, per split.
fn count(set: &LabelSet, images: &[(usize, Option<Split>)], dropped: &[bool]) -> (PerSplit, Vec<PerSplit>) {
    let mut per_split = [0; 4];
    let mut labels = vec![[0; 4]; set.classes.len()];
    for &(i, split) in images {
        let slot = split_slot(split);
        per_split[slot] += 1;
        for class in classes_of(&set.images[i], dropped) {
            if let Some(class) = labels.get_mut(class) {
                class[slot] += 1;
            }
        }
    }
    (per_split, labels)
}

/// Runs the plan: the images that stay (positions in `set.images`, in their
/// original order) with their split, and the counts. Nothing stays when the
/// preview has problems.
fn run(plan: &BalancePlan, set: &LabelSet) -> (Vec<(usize, Option<Split>)>, BalancePreview) {
    let seed = plan.seed.unwrap_or(DEFAULT_SEED);
    let all: Vec<(usize, Option<Split>)> = set.images.iter().enumerate().map(|(i, image)| (i, image.split)).collect();
    let (images_before, before) = count(set, &all, &[]);
    let mut preview = BalancePreview {
        seed,
        images_before,
        classes: set.classes.iter().zip(before).map(|(name, before)| ClassChange { name: name.clone(), before, after: [0; 4] }).collect(),
        problems: plan.problems(),
        ..Default::default()
    };
    for class in plan.drop_classes.iter().chain(&plan.filter.classes).filter(|c| !set.classes.contains(*c)) {
        preview.problems.push(format!("\"{class}\" isn't a class of this dataset."));
    }
    let dropped: Vec<bool> = set.classes.iter().map(|c| plan.drop_classes.contains(c)).collect();
    let labels_of = |class: &ClassChange| class.before.iter().sum::<u32>();
    if dropped.contains(&true) && !preview.classes.iter().zip(&dropped).any(|(class, dropped)| !dropped && labels_of(class) > 0) {
        preview.problems.push("Keep at least one class that has labels.".into());
    }
    if !preview.problems.is_empty() {
        // Nothing runs: the counts stay as they are (the chart shows every column whole).
        preview.images_after = preview.images_before;
        for class in &mut preview.classes {
            class.after = class.before;
        }
        return (Vec::new(), preview);
    }

    let mut rng = Rng(seed);
    // Start from the file order, so the result doesn't depend on how the labels were read.
    let mut order: Vec<usize> = (0..set.images.len()).collect();
    order.sort_by(|&a, &b| set.images[a].file.cmp(&set.images[b].file));

    if dropped.contains(&true) {
        preview.dropped_labels = preview.classes.iter().zip(&dropped).filter(|(_, dropped)| **dropped).map(|(class, _)| labels_of(class)).sum();
        let total = order.len();
        // Images that were unlabeled from the start stay: they are background images.
        order.retain(|&i| set.images[i].objects.is_empty() || classes_of(&set.images[i], &dropped).next().is_some());
        preview.removed_by_classes = (total - order.len()) as u32;
    }

    if !plan.filter.classes.is_empty() {
        let chosen: Vec<bool> = set.classes.iter().map(|c| plan.filter.classes.contains(c)).collect();
        let total = order.len();
        order.retain(|&i| {
            let has = classes_of(&set.images[i], &dropped).any(|class| chosen.get(class) == Some(&true));
            has == (plan.filter.mode == FilterMode::Keep)
        });
        preview.removed_by_filter = (total - order.len()) as u32;
    }

    if let Some(percent) = plan.subset_percent {
        let total = order.len();
        let mut kept = Vec::new();
        // From each split, so a small val or test set isn't lost by chance.
        for split in SPLITS {
            let mut group: Vec<usize> = order.iter().copied().filter(|&i| set.images[i].split == split).collect();
            rng.shuffle(&mut group);
            let take = (group.len() as u64 * u64::from(percent) + 50) / 100;
            group.truncate(take.max(1) as usize);
            kept.extend(group);
        }
        order = kept;
        preview.removed_by_subset = (total - order.len()) as u32;
    }

    if let Some(cap) = plan.class_cap {
        let total = order.len();
        let totals = class_totals(set, &order, &dropped);
        // Images with rare classes choose first, so the dominant classes fill what is left.
        rng.shuffle(&mut order);
        order.sort_by_cached_key(|&i| rarest_class(&set.images[i], &totals, &dropped));
        let mut labels = vec![0u32; set.classes.len()];
        order.retain(|&i| {
            let image = &set.images[i];
            let under_cap = classes_of(image, &dropped).any(|class| labels.get(class).is_some_and(|&n| n < cap));
            let unlabeled = !classes_of(image, &dropped).any(|class| class < labels.len());
            if under_cap || unlabeled {
                for class in classes_of(image, &dropped) {
                    if let Some(n) = labels.get_mut(class) {
                        *n += 1;
                    }
                }
            }
            under_cap || unlabeled
        });
        preview.removed_by_cap = (total - order.len()) as u32;
    }

    let mut kept: Vec<(usize, Option<Split>)> = match plan.split {
        Resplit::Keep => order.iter().map(|&i| (i, set.images[i].split)).collect(),
        Resplit::Ratios { train, val, test } => {
            rng.shuffle(&mut order);
            if plan.stratify {
                // Images sharing a rarest class sit together and are dealt out evenly.
                let totals = class_totals(set, &order, &dropped);
                order.sort_by_cached_key(|&i| rarest_class(&set.images[i], &totals, &dropped));
            }
            let splits = assign(order.len(), [train, val, test]);
            order.iter().copied().zip(splits.into_iter().map(Some)).collect()
        }
    };
    kept.sort_unstable_by_key(|&(i, _)| i);

    let (images_after, after) = count(set, &kept, &dropped);
    preview.images_after = images_after;
    for (class, after) in preview.classes.iter_mut().zip(after) {
        class.after = after;
    }
    if kept.is_empty() {
        preview.problems.push("No images are left.".into());
    }
    (kept, preview)
}

/// What `plan` does to `set`, counted exactly.
pub fn preview(plan: &BalancePlan, set: &LabelSet) -> BalancePreview {
    run(plan, set).1
}

/// Applies `plan`: the remaining images, in their original order, with their
/// new splits. Left-out classes lose their labels and leave the class list
/// (later ids move down). Fails on the preview's first problem.
pub fn apply(plan: &BalancePlan, set: LabelSet) -> Result<(LabelSet, BalancePreview), String> {
    let (kept, preview) = run(plan, &set);
    if let Some(problem) = preview.problems.first() {
        return Err(problem.clone());
    }
    // New id of each class; `None` for a left-out one.
    let mut next = 0;
    let new_id: Vec<Option<usize>> = set
        .classes
        .iter()
        .map(|class| {
            (!plan.drop_classes.contains(class)).then(|| {
                next += 1;
                next - 1
            })
        })
        .collect();
    let removed = set.classes.len() - next;

    let mut images: Vec<Option<ImageLabels>> = set.images.into_iter().map(Some).collect();
    let images = kept
        .into_iter()
        .filter_map(|(i, split)| {
            let mut image = images[i].take()?;
            image.split = split;
            if removed > 0 {
                image.objects.retain_mut(|object| match new_id.get(object.class) {
                    Some(Some(id)) => {
                        object.class = *id;
                        true
                    }
                    Some(None) => false,
                    // Unknown ids stay unknown.
                    None => true,
                });
            }
            Some(image)
        })
        .collect();
    let classes = set.classes.into_iter().zip(&new_id).filter(|(_, id)| id.is_some()).map(|(class, _)| class).collect();
    let declared_classes = set.declared_classes.map(|n| n.saturating_sub(removed));
    Ok((LabelSet { images, classes, warnings: set.warnings, declared_classes }, preview))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::core::labels::{BBox, Object};

    /// Classes a, b; one image per entry: its split and the classes of its objects.
    fn set(images: Vec<(Option<Split>, Vec<usize>)>) -> LabelSet {
        LabelSet {
            classes: vec!["a".into(), "b".into()],
            images: images
                .into_iter()
                .enumerate()
                .map(|(i, (split, objects))| ImageLabels {
                    file: PathBuf::from(format!("{i:03}.jpg")),
                    name: format!("{i:03}.jpg"),
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

    /// 80 images of a, then 20 of b, all in train.
    fn imbalanced() -> LabelSet {
        set((0..100).map(|i| (Some(Split::Train), vec![usize::from(i >= 80)])).collect())
    }

    #[test]
    fn splits_by_ratio_stratified_and_repeatable() {
        let data = imbalanced();
        let plan = BalancePlan { seed: Some(7), ..BalancePlan::new(1) };
        let p = preview(&plan, &data);
        assert!(p.can_run(), "{:?}", p.problems);
        assert_eq!((p.images_before, p.images_after), ([100, 0, 0, 0], [70, 20, 10, 0]));
        assert_eq!(p.classes[1].after, [14, 4, 2, 0], "the rare class keeps the ratio");
        assert_eq!(p.classes[0].after, [56, 16, 8, 0]);

        let (out, applied) = apply(&plan, data.clone()).unwrap();
        assert_eq!(applied, p);
        assert_eq!(out.images.len(), 100);
        assert!(out.images.windows(2).all(|w| w[0].name < w[1].name), "original order kept");
        let (again, _) = apply(&plan, data.clone()).unwrap();
        assert_eq!(out, again, "same seed, same split");
        let (other, _) = apply(&BalancePlan { seed: Some(8), ..plan.clone() }, data.clone()).unwrap();
        assert_ne!(out, other, "another seed, another split");

        // Without stratifying only the totals are exact.
        let p = preview(&BalancePlan { stratify: false, ..plan }, &data);
        assert_eq!(p.images_after, [70, 20, 10, 0]);
    }

    #[test]
    fn filters_by_class() {
        // a, b, a+b, empty.
        let data = set(vec![(None, vec![0]), (None, vec![1]), (None, vec![0, 1]), (None, vec![])]);
        let keep = BalancePlan { input: 1, filter: ClassFilter { mode: FilterMode::Keep, classes: vec!["b".into()] }, ..Default::default() };
        let (out, p) = apply(&keep, data.clone()).unwrap();
        assert_eq!(out.images.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["001.jpg", "002.jpg"]);
        assert_eq!((p.removed_by_filter, p.classes[0].after[3]), (2, 1), "other classes' labels stay on kept images");

        let remove = BalancePlan { filter: ClassFilter { mode: FilterMode::Remove, classes: vec!["b".into()] }, ..keep.clone() };
        let (out, _) = apply(&remove, data.clone()).unwrap();
        assert_eq!(out.images.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["000.jpg", "003.jpg"]);

        let unknown = BalancePlan { filter: ClassFilter { mode: FilterMode::Keep, classes: vec!["c".into()] }, ..keep.clone() };
        assert!(apply(&unknown, data.clone()).unwrap_err().contains("\"c\""));

        let both = BalancePlan { filter: ClassFilter { mode: FilterMode::Remove, classes: vec!["a".into(), "b".into()] }, subset_percent: Some(1), ..keep };
        let (out, _) = apply(&both, data.clone()).unwrap();
        assert_eq!(out.images.len(), 1, "a subset keeps at least one image");
        let nothing = set(vec![(None, vec![0])]);
        assert!(apply(&both, nothing).unwrap_err().contains("No images"));
    }

    #[test]
    fn leaves_out_classes() {
        // a, b, a+b, empty, a, a.
        let data = set(vec![(None, vec![0]), (None, vec![1]), (None, vec![0, 1]), (None, vec![]), (None, vec![0]), (None, vec![0])]);
        let plan = BalancePlan { input: 1, drop_classes: vec!["a".into()], ..Default::default() };
        let (out, p) = apply(&plan, data.clone()).unwrap();
        assert_eq!((out.classes.as_slice(), out.images.len()), (&["b".to_string()][..], 3));
        assert_eq!(out.images.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["001.jpg", "002.jpg", "003.jpg"], "the background image stays");
        assert!(out.images.iter().flat_map(|i| &i.objects).all(|o| o.class == 0), "b moved to id 0");
        assert_eq!(out.images[1].objects.len(), 1, "a's box is gone from the shared image");
        assert_eq!((p.dropped_labels, p.removed_by_classes), (4, 3));
        assert_eq!((p.classes[0].after, p.classes[1].after), ([0; 4], [0, 0, 0, 2]));

        // The cap doesn't count a left-out class: b has 2 labels, under a cap of 2.
        let capped = BalancePlan { class_cap: Some(2), ..plan.clone() };
        assert_eq!(preview(&capped, &data).removed_by_cap, 0);

        let all = BalancePlan { drop_classes: vec!["a".into(), "b".into()], ..plan.clone() };
        assert!(apply(&all, data.clone()).unwrap_err().contains("at least one class"));
        let unknown = BalancePlan { drop_classes: vec!["c".into()], ..plan };
        assert!(apply(&unknown, data).unwrap_err().contains("\"c\""));
    }

    #[test]
    fn subset_keeps_each_splits_share() {
        let images = (0..14).map(|i| (Some(if i < 10 { Split::Train } else { Split::Val }), vec![0])).collect();
        let plan = BalancePlan { input: 1, subset_percent: Some(50), ..Default::default() };
        let p = preview(&plan, &set(images));
        assert_eq!((p.images_after, p.removed_by_subset), ([5, 2, 0, 0], 7));
    }

    #[test]
    fn caps_dominant_classes() {
        // 10 of a, 2 of b, 1 with both, 1 empty.
        let mut images: Vec<(Option<Split>, Vec<usize>)> = (0..10).map(|_| (None, vec![0])).collect();
        images.extend([(None, vec![1]), (None, vec![1]), (None, vec![0, 1]), (None, vec![])]);
        let plan = BalancePlan { input: 1, class_cap: Some(3), ..Default::default() };
        let p = preview(&plan, &set(images));
        assert_eq!(p.classes[0].after[3], 3, "a is cut to the cap, counting the image it shares with b");
        assert_eq!(p.classes[1].after[3], 3, "b is under the cap and stays whole");
        assert_eq!((p.total_after(), p.removed_by_cap), (6, 8), "the unlabeled image stays");
    }
}
