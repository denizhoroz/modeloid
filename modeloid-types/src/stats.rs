//! Explore statistics and gallery pages: what the core computes and the
//! GUI draws.

use serde::{Deserialize, Serialize};

use crate::Split;

/// Bump when `ExploreStats` changes shape, so cached stats are recomputed.
pub const STATS_VERSION: u32 = 1;

/// Heatmap resolution: box centres are counted on a `HEATMAP_SIZE`² grid.
pub const HEATMAP_SIZE: usize = 32;

/// Histogram with `counts.len()` bins between consecutive `edges`.
/// Values outside the range go into the first / last bin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Histogram {
    pub edges: Vec<f64>,
    pub counts: Vec<u32>,
}

impl Histogram {
    pub fn linear(min: f64, max: f64, bins: usize) -> Self {
        let step = (max - min) / bins as f64;
        Histogram { edges: (0..=bins).map(|i| min + step * i as f64).collect(), counts: vec![0; bins] }
    }

    pub fn add(&mut self, value: f64) {
        if !value.is_finite() || self.counts.is_empty() {
            return;
        }
        let bins = self.counts.len();
        let (min, max) = (self.edges[0], self.edges[bins]);
        let pos = ((value - min) / (max - min) * bins as f64).floor();
        let index = (pos.max(0.0) as usize).min(bins - 1);
        self.counts[index] += 1;
    }

    pub fn total(&self) -> u32 {
        self.counts.iter().sum()
    }
}

/// Per-split numbers, in `SPLITS` order: train, val, test, no split.
pub type PerSplit = [u32; 4];

/// Split order used by every `PerSplit`.
pub const SPLITS: [Option<Split>; 4] = [Some(Split::Train), Some(Split::Val), Some(Split::Test), None];

pub fn split_slot(split: Option<Split>) -> usize {
    SPLITS.iter().position(|s| *s == split).unwrap_or(3)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClassStats {
    pub name: String,
    /// Objects of this class, per split.
    pub labels: PerSplit,
    /// Images containing at least one object of this class.
    pub images: u32,
}

impl ClassStats {
    pub fn total_labels(&self) -> u32 {
        self.labels.iter().sum()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploreStats {
    pub image_count: u32,
    pub label_count: u32,
    /// Images per split (`SPLITS` order).
    pub images: PerSplit,
    /// Images without any object, per split.
    pub empty_images: PerSplit,
    pub classes: Vec<ClassStats>,
    /// Objects per image; the last bin also holds everything above it.
    pub labels_per_image: Histogram,
    /// Box width and height as a fraction of the image's.
    pub box_width: Histogram,
    pub box_height: Histogram,
    /// `log2(width / height)`: 0 is square, +1 twice as wide, -1 twice as tall.
    pub aspect_ratio: Histogram,
    /// Objects by COCO size: small (< 32²px), medium (< 96²px), large.
    pub size_classes: [u32; 3],
    /// Box centres on a `HEATMAP_SIZE`² grid, row by row from the top left.
    pub heatmap: Vec<u32>,
    /// Most common image sizes, most frequent first.
    pub image_sizes: Vec<ImageSize>,
    /// Images whose size isn't in `image_sizes`.
    pub other_sizes: u32,
    /// Objects with segmentation outlines.
    pub polygon_count: u32,
    pub warnings: Vec<String>,
    pub warning_count: u32,
}

/// Which images a gallery page shows. Empty filter = all images.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GalleryFilter {
    /// Only images containing this class.
    pub class: Option<usize>,
    /// Only this split; `Some(None)` = images without a split.
    pub split: Option<Option<Split>>,
    /// Only images without objects.
    pub unlabeled: bool,
}

/// One object on a gallery image, in 0..1 coordinates of the image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GalleryObject {
    pub class: usize,
    /// x, y (top left), width, height.
    pub bbox: [f32; 4],
    pub polygon: Option<Vec<[f32; 2]>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GalleryImage {
    /// Position in the dataset's `LabelSet::images`; identifies the image file.
    pub index: usize,
    pub name: String,
    pub split: Option<Split>,
    pub width: u32,
    pub height: u32,
    pub objects: Vec<GalleryObject>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GalleryPage {
    /// Images matching the filter (all pages).
    pub total: usize,
    pub offset: usize,
    pub images: Vec<GalleryImage>,
}

