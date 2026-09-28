//! Health Check results and fixes: what the core finds and the GUI shows.

use serde::{Deserialize, Serialize};

use crate::Split;

/// Bump when `HealthReport` changes shape, so cached reports are dropped.
pub const HEALTH_VERSION: u32 = 1;

/// An image of the checked dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageRef {
    /// Position in the dataset's label set when checked (for previews).
    pub index: usize,
    /// Path relative to the dataset folder, with `/` separators.
    pub path: String,
    pub split: Option<Split>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageIssue {
    pub image: ImageRef,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoxIssue {
    pub image: ImageRef,
    /// Object number within the image.
    pub object: usize,
    pub class: usize,
    pub detail: String,
}

/// Images that are the same (`exact`: identical files) or look the same.
/// Sorted train → val → test → no split, then by path; the first is the one kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub exact: bool,
    pub images: Vec<ImageRef>,
}

impl DuplicateGroup {
    /// Copies sit in different splits: evaluation data leaks into training.
    pub fn crosses_splits(&self) -> bool {
        self.images.iter().any(|i| i.split != self.images[0].split)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthReport {
    pub version: u32,
    /// The dataset's `dataset.toml` stamp at check time; a different stamp means the report is stale.
    pub stamp: String,
    /// RFC 3339 time of the check.
    pub checked_at: String,
    pub image_count: u32,
    pub label_count: u32,
    pub classes: Vec<String>,
    /// Images that can't be decoded.
    pub corrupt_images: Vec<ImageIssue>,
    /// Images whose real size differs from the size the labels assume.
    pub size_mismatches: Vec<ImageIssue>,
    pub unlabeled_images: Vec<ImageRef>,
    /// Label files (or COCO entries) without an image, relative paths.
    pub orphan_labels: Vec<String>,
    /// Unreadable label lines and similar problems met while reading.
    pub label_warnings: Vec<String>,
    /// Boxes with no width, height or area.
    pub degenerate_boxes: Vec<BoxIssue>,
    /// Boxes reaching outside their image.
    pub out_of_bounds: Vec<BoxIssue>,
    /// Objects whose class id isn't in the dataset's class list.
    pub unknown_classes: Vec<BoxIssue>,
    pub duplicates: Vec<DuplicateGroup>,
}

/// Number of findings per kind (lists sent to the page may be shortened).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HealthCounts {
    pub corrupt_images: usize,
    pub size_mismatches: usize,
    pub unlabeled_images: usize,
    pub orphan_labels: usize,
    pub label_warnings: usize,
    pub degenerate_boxes: usize,
    pub out_of_bounds: usize,
    pub unknown_classes: usize,
    pub exact_groups: usize,
    pub near_groups: usize,
    /// Images a duplicate fix would remove (all but one per group).
    pub duplicate_copies: usize,
    pub leak_groups: usize,
}

impl HealthCounts {
    /// Findings that are problems (images without labels can be intentional).
    pub fn problems(&self) -> usize {
        self.corrupt_images
            + self.size_mismatches
            + self.orphan_labels
            + self.label_warnings
            + self.degenerate_boxes
            + self.out_of_bounds
            + self.unknown_classes
            + self.exact_groups
            + self.near_groups
    }
}

impl HealthReport {
    pub fn counts(&self) -> HealthCounts {
        HealthCounts {
            corrupt_images: self.corrupt_images.len(),
            size_mismatches: self.size_mismatches.len(),
            unlabeled_images: self.unlabeled_images.len(),
            orphan_labels: self.orphan_labels.len(),
            label_warnings: self.label_warnings.len(),
            degenerate_boxes: self.degenerate_boxes.len(),
            out_of_bounds: self.out_of_bounds.len(),
            unknown_classes: self.unknown_classes.len(),
            exact_groups: self.duplicates.iter().filter(|g| g.exact).count(),
            near_groups: self.duplicates.iter().filter(|g| !g.exact).count(),
            duplicate_copies: self.duplicates.iter().map(|g| g.images.len() - 1).sum(),
            leak_groups: self.duplicates.iter().filter(|g| g.crosses_splits()).count(),
        }
    }

    /// The same report with every list cut to `limit` entries (for display).
    pub fn truncated(&self, limit: usize) -> HealthReport {
        fn first<T: Clone>(items: &[T], limit: usize) -> Vec<T> {
            items.iter().take(limit).cloned().collect()
        }
        let mut report = self.clone();
        report.corrupt_images = first(&self.corrupt_images, limit);
        report.size_mismatches = first(&self.size_mismatches, limit);
        report.unlabeled_images = first(&self.unlabeled_images, limit);
        report.orphan_labels = first(&self.orphan_labels, limit);
        report.label_warnings = first(&self.label_warnings, limit);
        report.degenerate_boxes = first(&self.degenerate_boxes, limit);
        report.out_of_bounds = first(&self.out_of_bounds, limit);
        report.unknown_classes = first(&self.unknown_classes, limit);
        // Leaks first: they matter most.
        let mut groups = self.duplicates.clone();
        groups.sort_by_key(|g| (!g.crosses_splits(), !g.exact));
        report.duplicates = groups.into_iter().take(limit).collect();
        report
    }
}

/// Which fixes to apply. Fixing rewrites the dataset from its images and labels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthFixes {
    pub remove_corrupt: bool,
    pub drop_degenerate: bool,
    pub clip_out_of_bounds: bool,
    pub drop_unknown_classes: bool,
    /// For duplicates across splits, keep the copy in the earliest split (train first).
    pub remove_leaks: bool,
    pub remove_exact_duplicates: bool,
    pub remove_near_duplicates: bool,
    pub remove_unlabeled: bool,
}

impl HealthFixes {
    /// Fixes that are safe to apply without looking: they only remove what
    /// can't be right.
    pub fn recommended(counts: &HealthCounts) -> Self {
        HealthFixes {
            remove_corrupt: counts.corrupt_images > 0,
            drop_degenerate: counts.degenerate_boxes > 0,
            clip_out_of_bounds: counts.out_of_bounds > 0,
            drop_unknown_classes: counts.unknown_classes > 0,
            remove_leaks: counts.leak_groups > 0,
            remove_exact_duplicates: counts.exact_groups > 0,
            remove_near_duplicates: false,
            remove_unlabeled: false,
        }
    }

    pub fn any(&self) -> bool {
        *self != HealthFixes::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(index: usize, split: Option<Split>) -> ImageRef {
        ImageRef { index, path: format!("{index}.jpg"), split }
    }

    #[test]
    fn counts_and_leaks() {
        let report = HealthReport {
            version: HEALTH_VERSION,
            stamp: String::new(),
            checked_at: String::new(),
            image_count: 5,
            label_count: 0,
            classes: vec![],
            corrupt_images: vec![],
            size_mismatches: vec![],
            unlabeled_images: vec![image(4, None)],
            orphan_labels: vec![],
            label_warnings: vec![],
            degenerate_boxes: vec![],
            out_of_bounds: vec![],
            unknown_classes: vec![],
            duplicates: vec![
                DuplicateGroup { exact: true, images: vec![image(0, Some(Split::Train)), image(1, Some(Split::Train))] },
                DuplicateGroup { exact: false, images: vec![image(2, Some(Split::Train)), image(3, Some(Split::Test))] },
            ],
        };
        let counts = report.counts();
        assert_eq!((counts.exact_groups, counts.near_groups, counts.leak_groups, counts.duplicate_copies), (1, 1, 1, 2));
        assert_eq!(counts.problems(), 2, "unlabeled images aren't counted as problems");
        assert!(report.truncated(1).duplicates[0].crosses_splits(), "leaks listed first");

        let fixes = HealthFixes::recommended(&counts);
        assert!(fixes.remove_leaks && fixes.remove_exact_duplicates && !fixes.remove_near_duplicates);
        assert!(fixes.any() && !HealthFixes::default().any());
    }
}
