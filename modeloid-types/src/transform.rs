//! Transform: augmented copies of a dataset's images, made with
//! Albumentations in the Python environment. Which images (`TransformPlan`:
//! splits, classes, share, copies per image), which augmentations
//! (`AUGMENTATIONS`, one `AugRow` each) and what it would give
//! (`TransformPreview`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::stats::PerSplit;

/// Most augmented copies of one image.
pub const MAX_COPIES: u32 = 20;

/// What kind of value a setting takes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamKind {
    /// A random value is drawn from `low` to `high` each time.
    Pair,
    Number,
    Choice(&'static [&'static str]),
}

/// A setting's default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamDefault {
    Pair(f64, f64),
    Number(f64),
    Choice(&'static str),
}

/// One setting of an augmentation (an Albumentations argument).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AugParam {
    /// Albumentations' argument name, e.g. "blur_limit".
    pub key: &'static str,
    pub label: &'static str,
    pub kind: ParamKind,
    pub min: f64,
    pub max: f64,
    /// Whole numbers only.
    pub integer: bool,
    /// Odd whole numbers only (kernel sizes).
    pub odd: bool,
    pub default: ParamDefault,
}

const fn pair(key: &'static str, label: &'static str, min: f64, max: f64, low: f64, high: f64) -> AugParam {
    AugParam { key, label, kind: ParamKind::Pair, min, max, integer: false, odd: false, default: ParamDefault::Pair(low, high) }
}

const fn number(key: &'static str, label: &'static str, min: f64, max: f64, value: f64) -> AugParam {
    AugParam { key, label, kind: ParamKind::Number, min, max, integer: false, odd: false, default: ParamDefault::Number(value) }
}

const fn choice(key: &'static str, label: &'static str, options: &'static [&'static str]) -> AugParam {
    AugParam { key, label, kind: ParamKind::Choice(options), min: 0.0, max: 0.0, integer: false, odd: false, default: ParamDefault::Choice(options[0]) }
}

const fn int(mut p: AugParam) -> AugParam {
    p.integer = true;
    p
}

const fn odd(mut p: AugParam) -> AugParam {
    p.integer = true;
    p.odd = true;
    p
}

/// What empty corners left by rotating or shifting are filled with.
const BORDER: AugParam = choice("border_mode", "Fill", &["constant", "reflect", "replicate"]);

/// An augmentation the table offers (an Albumentations transform).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Augmentation {
    /// Albumentations' class name, e.g. "HorizontalFlip".
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub tip: &'static str,
    /// Chance of being applied to a copy.
    pub p: f64,
    /// Used unless switched off.
    pub on: bool,
    /// Moves pixels, so labels move with it.
    pub spatial: bool,
    pub params: &'static [AugParam],
}

const fn aug(key: &'static str, label: &'static str, group: &'static str, tip: &'static str, p: f64, params: &'static [AugParam]) -> Augmentation {
    Augmentation { key, label, group, tip, p, on: false, spatial: false, params }
}

const fn on(mut a: Augmentation) -> Augmentation {
    a.on = true;
    a
}

const fn spatial(mut a: Augmentation) -> Augmentation {
    a.spatial = true;
    a
}

const SIZE: &str = "Size & crop";
const FLIP: &str = "Flip & rotate";
const GEOMETRY: &str = "Geometry";
const COLOR: &str = "Color";
const BLUR: &str = "Blur & noise";
const DROPOUT: &str = "Dropout";
const WEATHER: &str = "Weather";

/// Every augmentation, in the order they are applied.
pub const AUGMENTATIONS: &[Augmentation] = &[
    spatial(aug("Resize", "Resize", SIZE, "Every copy gets exactly this size; the aspect ratio can change.", 1.0, &[
        int(number("height", "Height (px)", 1.0, 8192.0, 640.0)),
        int(number("width", "Width (px)", 1.0, 8192.0, 640.0)),
    ])),
    spatial(aug("LongestMaxSize", "Fit to size", SIZE, "Scales the image so its long side is this many pixels, keeping the aspect ratio.", 1.0, &[
        int(number("max_size", "Long side (px)", 1.0, 8192.0, 1024.0)),
    ])),
    spatial(aug("RandomResizedCrop", "Random resized crop", SIZE, "Crops a random part (a share of the area) and resizes it to the given size. Labels cut off at the edge are clipped.", 0.5, &[
        int(number("height", "Height (px)", 1.0, 8192.0, 640.0)),
        int(number("width", "Width (px)", 1.0, 8192.0, 640.0)),
        pair("scale", "Area share", 0.01, 1.0, 0.5, 1.0),
    ])),
    spatial(aug("RandomCrop", "Random crop", SIZE, "Crops a random part of this size; smaller images are padded first.", 0.5, &[
        int(number("height", "Height (px)", 1.0, 8192.0, 320.0)),
        int(number("width", "Width (px)", 1.0, 8192.0, 320.0)),
    ])),
    spatial(aug("BBoxSafeRandomCrop", "Box-safe crop", SIZE, "Crops a random part that still holds every box.", 0.5, &[
        number("erosion_rate", "Erosion", 0.0, 1.0, 0.0),
    ])),
    on(spatial(aug("HorizontalFlip", "Horizontal flip", FLIP, "Mirrors the image left to right.", 0.5, &[]))),
    spatial(aug("VerticalFlip", "Vertical flip", FLIP, "Turns the image upside down.", 0.5, &[])),
    spatial(aug("RandomRotate90", "Rotate 90°", FLIP, "Rotates by 0, 90, 180 or 270 degrees.", 0.5, &[])),
    spatial(aug("Transpose", "Transpose", FLIP, "Swaps rows and columns (a flip along the diagonal).", 0.5, &[])),
    spatial(aug("Rotate", "Rotate", FLIP, "Rotates by a random angle in degrees.", 0.5, &[pair("limit", "Angle (°)", -180.0, 180.0, -15.0, 15.0), BORDER])),
    on(spatial(aug("Affine", "Affine", GEOMETRY, "Zooms, shifts, rotates and shears in one step.", 0.5, &[
        pair("scale", "Zoom", 0.1, 3.0, 0.9, 1.1),
        pair("translate_percent", "Shift (share)", -0.5, 0.5, -0.1, 0.1),
        pair("rotate", "Rotation (°)", -180.0, 180.0, -10.0, 10.0),
        pair("shear", "Shear (°)", -45.0, 45.0, -5.0, 5.0),
        BORDER,
    ]))),
    spatial(aug("Perspective", "Perspective", GEOMETRY, "Tilts the image as if seen from another angle.", 0.5, &[pair("scale", "Strength", 0.0, 0.5, 0.05, 0.1)])),
    on(aug("RandomBrightnessContrast", "Brightness & contrast", COLOR, "Changes brightness and contrast by up to the given share.", 0.5, &[
        pair("brightness_limit", "Brightness", -1.0, 1.0, -0.2, 0.2),
        pair("contrast_limit", "Contrast", -1.0, 1.0, -0.2, 0.2),
    ])),
    aug("HueSaturationValue", "Hue, saturation, value", COLOR, "Shifts hue, saturation and brightness.", 0.5, &[
        pair("hue_shift_limit", "Hue", -180.0, 180.0, -20.0, 20.0),
        pair("sat_shift_limit", "Saturation", -255.0, 255.0, -30.0, 30.0),
        pair("val_shift_limit", "Value", -255.0, 255.0, -20.0, 20.0),
    ]),
    aug("ColorJitter", "Color jitter", COLOR, "Changes brightness, contrast and saturation by a factor (1 = unchanged) and shifts the hue.", 0.5, &[
        pair("brightness", "Brightness", 0.0, 3.0, 0.8, 1.2),
        pair("contrast", "Contrast", 0.0, 3.0, 0.8, 1.2),
        pair("saturation", "Saturation", 0.0, 3.0, 0.8, 1.2),
        pair("hue", "Hue", -0.5, 0.5, -0.05, 0.05),
    ]),
    aug("RandomGamma", "Gamma", COLOR, "Brightens or darkens mid-tones; 100 = unchanged.", 0.5, &[pair("gamma_limit", "Gamma", 1.0, 400.0, 80.0, 120.0)]),
    aug("CLAHE", "CLAHE", COLOR, "Local contrast equalization; brings out detail in dark or flat areas.", 0.5, &[pair("clip_limit", "Clip limit", 1.0, 40.0, 1.0, 4.0)]),
    aug("RGBShift", "RGB shift", COLOR, "Shifts the red, green and blue channels separately.", 0.5, &[
        pair("r_shift_limit", "Red", -255.0, 255.0, -20.0, 20.0),
        pair("g_shift_limit", "Green", -255.0, 255.0, -20.0, 20.0),
        pair("b_shift_limit", "Blue", -255.0, 255.0, -20.0, 20.0),
    ]),
    aug("ChannelShuffle", "Channel shuffle", COLOR, "Puts the color channels in a random order.", 0.5, &[]),
    aug("ToGray", "Grayscale", COLOR, "Makes the image gray (still three channels). At 100% every copy is gray.", 0.5, &[]),
    aug("ToSepia", "Sepia", COLOR, "A brownish old-photo tone.", 0.5, &[]),
    aug("Equalize", "Equalize", COLOR, "Spreads the brightness histogram evenly.", 0.5, &[]),
    aug("InvertImg", "Invert", COLOR, "Turns the image into its negative.", 0.5, &[]),
    aug("Posterize", "Posterize", COLOR, "Keeps only this many bits per color channel.", 0.5, &[int(pair("num_bits", "Bits", 1.0, 8.0, 4.0, 8.0))]),
    aug("Solarize", "Solarize", COLOR, "Inverts the pixels above a brightness threshold (a share of white).", 0.5, &[pair("threshold_range", "Threshold", 0.0, 1.0, 0.5, 0.5)]),
    aug("Blur", "Blur", BLUR, "Box blur; the kernel size in pixels, odd numbers.", 0.5, &[odd(pair("blur_limit", "Kernel (px)", 3.0, 99.0, 3.0, 7.0))]),
    aug("GaussianBlur", "Gaussian blur", BLUR, "Soft blur; the kernel size in pixels, odd numbers.", 0.5, &[odd(pair("blur_limit", "Kernel (px)", 3.0, 99.0, 3.0, 7.0))]),
    aug("MotionBlur", "Motion blur", BLUR, "Blur as from a moving camera; the kernel size in pixels, odd numbers.", 0.5, &[odd(pair("blur_limit", "Kernel (px)", 3.0, 99.0, 3.0, 7.0))]),
    aug("MedianBlur", "Median blur", BLUR, "Smooths noise while keeping edges; the kernel size in pixels, odd numbers.", 0.5, &[odd(pair("blur_limit", "Kernel (px)", 3.0, 99.0, 3.0, 7.0))]),
    aug("GaussNoise", "Gaussian noise", BLUR, "Adds noise; its strength as a share of full brightness.", 0.5, &[pair("std_range", "Strength", 0.0, 1.0, 0.02, 0.1)]),
    aug("ISONoise", "Camera noise", BLUR, "Sensor noise like a high-ISO photo.", 0.5, &[
        pair("color_shift", "Color shift", 0.0, 1.0, 0.01, 0.05),
        pair("intensity", "Intensity", 0.0, 2.0, 0.1, 0.5),
    ]),
    aug("ImageCompression", "JPEG compression", BLUR, "Compression artifacts; lower quality means more.", 0.5, &[int(pair("quality_range", "Quality", 1.0, 100.0, 60.0, 100.0))]),
    aug("Sharpen", "Sharpen", BLUR, "Sharpens edges.", 0.5, &[
        pair("alpha", "Strength", 0.0, 1.0, 0.2, 0.5),
        pair("lightness", "Lightness", 0.0, 5.0, 0.5, 1.0),
    ]),
    aug("Downscale", "Downscale", BLUR, "Lowers the resolution and scales back up, so the image looks blocky.", 0.5, &[pair("scale_range", "Scale", 0.01, 0.99, 0.5, 0.9)]),
    aug("CoarseDropout", "Coarse dropout", DROPOUT, "Blacks out random rectangles; their size as a share of the image.", 0.5, &[
        int(pair("num_holes_range", "Holes", 1.0, 100.0, 1.0, 8.0)),
        pair("hole_height_range", "Height (share)", 0.0, 1.0, 0.05, 0.1),
        pair("hole_width_range", "Width (share)", 0.0, 1.0, 0.05, 0.1),
    ]),
    aug("RandomFog", "Fog", WEATHER, "Adds fog; its thickness from 0 to 1.", 0.3, &[pair("fog_coef_range", "Thickness", 0.0, 1.0, 0.3, 0.6)]),
    aug("RandomRain", "Rain", WEATHER, "Adds rain streaks.", 0.3, &[choice("rain_type", "Rain", &["default", "drizzle", "heavy", "torrential"])]),
    aug("RandomSnow", "Snow", WEATHER, "Bleaches parts of the image like snow; how much from 0 to 1.", 0.3, &[pair("snow_point_range", "Amount", 0.0, 1.0, 0.1, 0.3)]),
    aug("RandomSunFlare", "Sun flare", WEATHER, "Adds a lens flare in the top half.", 0.3, &[]),
    aug("RandomShadow", "Shadow", WEATHER, "Casts shadows on the bottom half.", 0.3, &[]),
];

pub fn augmentation(key: &str) -> Option<&'static Augmentation> {
    AUGMENTATIONS.iter().find(|a| a.key == key)
}

/// A setting's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AugValue {
    Pair([f64; 2]),
    Number(f64),
    Text(String),
}

impl AugValue {
    fn default_for(param: &AugParam) -> AugValue {
        match param.default {
            ParamDefault::Pair(low, high) => AugValue::Pair([low, high]),
            ParamDefault::Number(n) => AugValue::Number(n),
            ParamDefault::Choice(c) => AugValue::Text(c.to_string()),
        }
    }

    /// Short text, e.g. "-0.2 to 0.2".
    pub fn label(&self) -> String {
        fn num(n: f64) -> String {
            let text = format!("{n:.4}");
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        match self {
            AugValue::Pair([low, high]) if low == high => num(*low),
            AugValue::Pair([low, high]) => format!("{} to {}", num(*low), num(*high)),
            AugValue::Number(n) => num(*n),
            AugValue::Text(t) => t.clone(),
        }
    }
}

/// A row of the augmentation table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AugRow {
    /// `Augmentation::key`.
    pub key: String,
    pub enabled: bool,
    /// Chance of being applied to a copy, 0 to 1.
    pub p: f64,
    #[serde(default)]
    pub params: BTreeMap<String, AugValue>,
}

impl AugRow {
    pub fn default_for(a: &Augmentation) -> AugRow {
        AugRow {
            key: a.key.to_string(),
            enabled: a.on,
            p: a.p,
            params: a.params.iter().map(|p| (p.key.to_string(), AugValue::default_for(p))).collect(),
        }
    }

    /// `param`'s value, or its default when missing.
    pub fn value(&self, param: &AugParam) -> AugValue {
        self.params.get(param.key).cloned().unwrap_or_else(|| AugValue::default_for(param))
    }

    /// What's wrong with this row, if it is used.
    pub fn problems(&self) -> Vec<String> {
        let Some(a) = augmentation(&self.key) else { return vec![format!("\"{}\" isn't an augmentation.", self.key)] };
        if !self.enabled {
            return Vec::new();
        }
        let mut problems = Vec::new();
        if !(0.0..=1.0).contains(&self.p) {
            problems.push(format!("{}: the chance must be from 0% to 100%.", a.label));
        }
        for param in a.params {
            if let Some(problem) = value_problem(param, &self.value(param)) {
                problems.push(format!("{} · {}: {problem}", a.label, param.label));
            }
        }
        problems
    }
}

fn value_problem(param: &AugParam, value: &AugValue) -> Option<String> {
    let number_problem = |n: f64| -> Option<String> {
        if !n.is_finite() || n < param.min || n > param.max {
            Some(format!("from {} to {}.", param.min, param.max))
        } else if param.odd && (n.fract() != 0.0 || n % 2.0 == 0.0) {
            Some("odd whole numbers only.".into())
        } else if param.integer && n.fract() != 0.0 {
            Some("whole numbers only.".into())
        } else {
            None
        }
    };
    match (param.kind, value) {
        (ParamKind::Pair, AugValue::Pair([low, high])) => {
            number_problem(*low).or_else(|| number_problem(*high)).or_else(|| (low > high).then(|| "the first value must not be above the second.".into()))
        }
        (ParamKind::Number, AugValue::Number(n)) => number_problem(*n),
        (ParamKind::Choice(options), AugValue::Text(t)) if options.contains(&t.as_str()) => None,
        (ParamKind::Choice(options), _) => Some(format!("one of {}.", options.join(", "))),
        _ => Some("not the right kind of value.".into()),
    }
}

/// The default table: every augmentation, the usual ones switched on.
pub fn default_augmentations() -> Vec<AugRow> {
    AUGMENTATIONS.iter().map(AugRow::default_for).collect()
}

/// What Transform is asked to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransformPlan {
    /// Input dataset id.
    pub input: u32,
    /// Which splits' images are augmented: train, val, test, no split.
    pub splits: [bool; 4],
    /// Only images with at least one of these classes; empty = every image.
    #[serde(default)]
    pub classes: Vec<String>,
    /// Share of the matching images that get copies, 1 to 100.
    pub percent: u32,
    /// Augmented copies per chosen image.
    pub copies: u32,
    /// The chosen images stay next to their copies; otherwise only the copies do.
    pub keep_originals: bool,
    /// Seed for the choice of images and the augmentations; `None` = the workspace's.
    #[serde(default)]
    pub seed: Option<u64>,
    pub augmentations: Vec<AugRow>,
}

impl TransformPlan {
    /// Copies of a share of the training images, with the default augmentations.
    pub fn new(input: u32) -> TransformPlan {
        TransformPlan {
            input,
            splits: [true, false, false, false],
            classes: Vec::new(),
            percent: 100,
            copies: 1,
            keep_originals: true,
            seed: None,
            augmentations: default_augmentations(),
        }
    }

    /// The rows used, in the order they are applied.
    pub fn enabled(&self) -> Vec<&AugRow> {
        AUGMENTATIONS.iter().filter_map(|a| self.augmentations.iter().find(|r| r.key == a.key && r.enabled)).collect()
    }

    /// Problems with the plan itself (the dataset's classes are checked by the core).
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.splits.contains(&true) {
            problems.push("Choose at least one split to augment.".into());
        }
        if !(1..=100).contains(&self.percent) {
            problems.push("Share of images: 1% to 100%.".into());
        }
        if !(1..=MAX_COPIES).contains(&self.copies) {
            problems.push(format!("Copies per image: 1 to {MAX_COPIES}."));
        }
        if self.enabled().is_empty() {
            problems.push("Choose at least one augmentation.".into());
        }
        problems.extend(self.augmentations.iter().flat_map(AugRow::problems));
        problems
    }
}

/// What a plan would give, counted from the dataset's labels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TransformPreview {
    /// The seed used (the plan's or the workspace's).
    pub seed: u64,
    /// The dataset's classes, for the class choice.
    pub classes: Vec<String>,
    pub images_before: PerSplit,
    pub images_after: PerSplit,
    /// Images in the chosen splits and classes.
    pub matching: u32,
    /// Of those, the ones that get copies.
    pub chosen: u32,
    /// Augmented images added.
    pub added: u32,
    /// The dataset has outlines (they are transformed with the image).
    pub has_polygons: bool,
    pub problems: Vec<String>,
}

impl TransformPreview {
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
    fn defaults_are_valid() {
        let plan = TransformPlan::new(1);
        assert!(plan.problems().is_empty(), "{:?}", plan.problems());
        let keys: Vec<&str> = plan.enabled().iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["HorizontalFlip", "Affine", "RandomBrightnessContrast"]);
        let mut seen = std::collections::HashSet::new();
        assert!(AUGMENTATIONS.iter().all(|a| seen.insert(a.key)), "keys are unique");
    }

    #[test]
    fn checks_rows() {
        let mut blur = AugRow::default_for(augmentation("Blur").unwrap());
        blur.enabled = true;
        blur.params.insert("blur_limit".into(), AugValue::Pair([3.0, 8.0]));
        assert!(blur.problems()[0].contains("odd"));
        blur.params.insert("blur_limit".into(), AugValue::Pair([7.0, 3.0]));
        assert!(blur.problems()[0].contains("first value"));
        blur.enabled = false;
        assert!(blur.problems().is_empty(), "unused rows aren't checked");

        let mut resize = AugRow::default_for(augmentation("Resize").unwrap());
        resize.enabled = true;
        resize.p = 1.5;
        resize.params.insert("height".into(), AugValue::Number(0.0));
        assert_eq!(resize.problems().len(), 2);

        let mut rain = AugRow::default_for(augmentation("RandomRain").unwrap());
        rain.enabled = true;
        rain.params.insert("rain_type".into(), AugValue::Text("hail".into()));
        assert!(rain.problems()[0].contains("one of"));
    }

    #[test]
    fn plan_needs_splits_and_augmentations() {
        let mut plan = TransformPlan::new(1);
        plan.splits = [false; 4];
        plan.copies = 0;
        for row in &mut plan.augmentations {
            row.enabled = false;
        }
        assert_eq!(plan.problems().len(), 3);
        assert_eq!(AugValue::Pair([-0.2, 0.2]).label(), "-0.2 to 0.2");
        assert_eq!(AugValue::Pair([0.5, 0.5]).label(), "0.5");
    }

    #[test]
    fn values_survive_saving() {
        let row = AugRow::default_for(augmentation("Rotate").unwrap());
        let back: AugRow = toml::from_str(&toml::to_string(&row).unwrap()).unwrap();
        assert_eq!(row, back);
    }
}
