//! Split & Balance: leave classes out, filter images by class, take a random
//! subset, cap dominant classes and split into train / val / test. The steps
//! run in that order, each on what the one before left.

use serde::{Deserialize, Serialize};

use crate::stats::PerSplit;

/// Seed used when neither the plan nor a workspace gives one.
pub const DEFAULT_SEED: u64 = 42;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    /// Keep only images containing one of the classes.
    #[default]
    Keep,
    /// Remove images containing one of the classes.
    Remove,
}

/// Which images stay, by the classes they contain. No classes = no filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassFilter {
    pub mode: FilterMode,
    pub classes: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resplit {
    /// Images stay in the split they are in.
    #[default]
    Keep,
    /// Every image is assigned again, in these percentages (they add up to 100).
    Ratios { train: u32, val: u32, test: u32 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancePlan {
    /// Input dataset id.
    pub input: u32,
    /// Classes to leave out: their labels are deleted, they leave the class
    /// list, and images left without any label are removed.
    #[serde(default)]
    pub drop_classes: Vec<String>,
    #[serde(default)]
    pub filter: ClassFilter,
    /// Keep a random share of the images (1..=100), taken from each split.
    #[serde(default)]
    pub subset_percent: Option<u32>,
    /// Undersample dominant classes: images are left out once all their
    /// classes have this many labels. A class can still end above the cap
    /// through images kept for another class.
    #[serde(default)]
    pub class_cap: Option<u32>,
    #[serde(default)]
    pub split: Resplit,
    /// With `Resplit::Ratios`: give each split a similar class mix (images
    /// are grouped by their rarest class).
    #[serde(default)]
    pub stratify: bool,
    /// Seed of every random choice; `None` = the workspace's seed.
    #[serde(default)]
    pub seed: Option<u64>,
}

impl BalancePlan {
    /// The usual start: a stratified 70 / 20 / 10 split, nothing removed.
    pub fn new(input: u32) -> Self {
        BalancePlan { input, split: Resplit::Ratios { train: 70, val: 20, test: 10 }, stratify: true, ..Default::default() }
    }

    /// Problems that stop the plan from running, as messages.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if matches!(self.subset_percent, Some(p) if p == 0 || p > 100) {
            problems.push("The subset must be between 1 and 100%.".into());
        }
        if self.class_cap == Some(0) {
            problems.push("The cap must be at least 1 label per class.".into());
        }
        if let Resplit::Ratios { train, val, test } = self.split {
            let sum = u64::from(train) + u64::from(val) + u64::from(test);
            if sum != 100 {
                problems.push(format!("The split percentages add up to {sum}%, not 100%."));
            } else if train == 0 {
                problems.push("Train needs more than 0%.".into());
            }
        }
        let unchanged = self.drop_classes.is_empty()
            && self.filter.classes.is_empty()
            && self.subset_percent.is_none()
            && self.class_cap.is_none()
            && self.split == Resplit::Keep;
        if unchanged {
            problems.push("Nothing would change: no class left out, no subset, no cap and no new split.".into());
        }
        problems
    }
}

/// Labels of one class before and after, per split.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassChange {
    pub name: String,
    pub before: PerSplit,
    pub after: PerSplit,
}

/// What a plan does to a dataset, counted exactly (the plan is run in memory).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalancePreview {
    /// The seed used.
    pub seed: u64,
    pub images_before: PerSplit,
    pub images_after: PerSplit,
    /// Every class of the dataset, in id order (left-out ones have no labels after).
    pub classes: Vec<ClassChange>,
    /// Labels of left-out classes.
    pub dropped_labels: u32,
    /// Images that only had labels of left-out classes.
    pub removed_by_classes: u32,
    pub removed_by_filter: u32,
    pub removed_by_subset: u32,
    pub removed_by_cap: u32,
    /// Plan problems (see `BalancePlan::problems`) and ones found on the
    /// data. With any, `*_after` and `removed_*` are zero.
    pub problems: Vec<String>,
}

impl BalancePreview {
    pub fn can_run(&self) -> bool {
        self.problems.is_empty()
    }

    pub fn total_before(&self) -> u32 {
        self.images_before.iter().sum()
    }

    pub fn total_after(&self) -> u32 {
        self.images_after.iter().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plan_problems() {
        assert!(BalancePlan::new(1).problems().is_empty());
        assert!(BalancePlan::default().problems()[0].contains("Nothing would change"));

        let plan = BalancePlan {
            subset_percent: Some(0),
            class_cap: Some(0),
            split: Resplit::Ratios { train: 70, val: 20, test: 20 },
            ..Default::default()
        };
        let problems = plan.problems();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[2].contains("110%"));

        let drop_only = BalancePlan { drop_classes: vec!["cat".into()], ..Default::default() };
        assert!(drop_only.problems().is_empty());

        let no_train =BalancePlan { split: Resplit::Ratios { train: 0, val: 50, test: 50 }, ..Default::default() };
        assert!(no_train.problems()[0].contains("Train"));
    }
}
