//! Training parameters: a named, reusable recipe of model and training
//! settings, saved in the workspace as `training/parameters/<name>.toml`. The
//! dataset isn't part of it; Train Model picks that.
//!
//! The settings belong to one training framework (`framework = "..."` in the
//! file). Only Ultralytics exists so far; its field names and defaults are
//! Ultralytics' own, so training can pass them straight through.

use serde::{Deserialize, Serialize};

use crate::validate_dataset_name;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterSet {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// RFC 3339, set by the core when saved.
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub updated: String,
    #[serde(flatten)]
    pub params: FrameworkParams,
}

/// Settings of one training framework.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "framework", rename_all = "kebab-case")]
pub enum FrameworkParams {
    Ultralytics(UltralyticsParams),
}

impl Default for FrameworkParams {
    fn default() -> Self {
        FrameworkParams::Ultralytics(UltralyticsParams::default())
    }
}

impl ParameterSet {
    /// A new set with default settings.
    pub fn new(name: impl Into<String>) -> Self {
        ParameterSet {
            name: name.into(),
            description: None,
            created: String::new(),
            updated: String::new(),
            params: FrameworkParams::default(),
        }
    }

    /// One line for tables, e.g. "YOLO11n · detect".
    pub fn summary(&self) -> String {
        match &self.params {
            FrameworkParams::Ultralytics(p) => p.summary(),
        }
    }

    /// Everything that stops this set from being saved; empty when it's fine.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if let Err(e) = validate_dataset_name(&self.name) {
            problems.push(e);
        }
        match &self.params {
            FrameworkParams::Ultralytics(p) => problems.extend(p.problems()),
        }
        problems
    }

    /// The same settings, with a copy's name and no timestamps.
    pub fn duplicate(&self) -> Self {
        ParameterSet { name: format!("{}-copy", self.name), created: String::new(), updated: String::new(), ..self.clone() }
    }

    /// Whether the settings (name, description, params) differ; timestamps don't count.
    pub fn same_settings(&self, other: &ParameterSet) -> bool {
        self.name == other.name && self.description == other.description && self.params == other.params
    }
}

// ---------- Ultralytics ----------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UltralyticsParams {
    pub model: ModelConfig,
    pub training: TrainingConfig,
    pub schedule: ScheduleConfig,
    pub augmentation: AugmentationConfig,
    pub loss: LossConfig,
    pub run: RunConfig,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Task {
    #[default]
    Detect,
    Segment,
    Classify,
    Pose,
    Obb,
}

impl Task {
    pub const ALL: [Task; 5] = [Task::Detect, Task::Segment, Task::Classify, Task::Pose, Task::Obb];

    pub fn label(self) -> &'static str {
        match self {
            Task::Detect => "Detect",
            Task::Segment => "Segment",
            Task::Classify => "Classify",
            Task::Pose => "Pose",
            Task::Obb => "Oriented boxes",
        }
    }

    /// Ultralytics' name, e.g. "detect".
    pub fn id(self) -> &'static str {
        match self {
            Task::Detect => "detect",
            Task::Segment => "segment",
            Task::Classify => "classify",
            Task::Pose => "pose",
            Task::Obb => "obb",
        }
    }

    /// Suffix of the model file, e.g. "-seg".
    fn suffix(self) -> &'static str {
        match self {
            Task::Detect => "",
            Task::Segment => "-seg",
            Task::Classify => "-cls",
            Task::Pose => "-pose",
            Task::Obb => "-obb",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    #[default]
    Yolo11,
    Yolov8,
    Yolo12,
    RtDetr,
}

impl Family {
    pub const ALL: [Family; 4] = [Family::Yolo11, Family::Yolov8, Family::Yolo12, Family::RtDetr];

    pub fn label(self) -> &'static str {
        match self {
            Family::Yolo11 => "YOLO11",
            Family::Yolov8 => "YOLOv8",
            Family::Yolo12 => "YOLO12",
            Family::RtDetr => "RT-DETR",
        }
    }

    /// Sizes this family comes in.
    pub fn sizes(self) -> &'static [ModelSize] {
        match self {
            Family::RtDetr => &[ModelSize::L, ModelSize::X],
            _ => &ModelSize::ALL,
        }
    }

    /// Tasks it has pretrained weights for.
    pub fn pretrained_tasks(self) -> &'static [Task] {
        match self {
            Family::Yolo11 | Family::Yolov8 => &Task::ALL,
            Family::Yolo12 | Family::RtDetr => &[Task::Detect],
        }
    }

    /// Tasks it can train at all (from scratch included).
    pub fn tasks(self) -> &'static [Task] {
        match self {
            Family::RtDetr => &[Task::Detect],
            _ => &Task::ALL,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelSize {
    #[default]
    N,
    S,
    M,
    L,
    X,
}

impl ModelSize {
    pub const ALL: [ModelSize; 5] = [ModelSize::N, ModelSize::S, ModelSize::M, ModelSize::L, ModelSize::X];

    /// "n", "s", ...
    pub fn id(self) -> &'static str {
        match self {
            ModelSize::N => "n",
            ModelSize::S => "s",
            ModelSize::M => "m",
            ModelSize::L => "l",
            ModelSize::X => "x",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModelSize::N => "Nano",
            ModelSize::S => "Small",
            ModelSize::M => "Medium",
            ModelSize::L => "Large",
            ModelSize::X => "Extra large",
        }
    }
}

/// Where the starting weights come from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Weights {
    /// Pretrained on COCO (or ImageNet for classify), downloaded by Ultralytics.
    #[default]
    Pretrained,
    /// Random start from the model's architecture file.
    FromScratch,
    /// A weights (.pt) or architecture (.yaml) file, e.g. an earlier training run's best.pt.
    File(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    pub task: Task,
    pub family: Family,
    pub size: ModelSize,
    pub weights: Weights,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Batch {
    /// Ultralytics picks the largest batch that fits ~60% of GPU memory (`batch = -1`).
    Auto,
    Size(u32),
}

impl Batch {
    /// Images per batch; `None` means auto.
    pub fn size(self) -> Option<u32> {
        match self {
            Batch::Auto => None,
            Batch::Size(n) => Some(n),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Optimizer {
    /// Ultralytics chooses by run length (AdamW for short runs, SGD for long ones).
    #[default]
    Auto,
    Sgd,
    Adam,
    AdamW,
    NAdam,
    RAdam,
    RmsProp,
}

impl Optimizer {
    pub const ALL: [Optimizer; 7] =
        [Optimizer::Auto, Optimizer::Sgd, Optimizer::Adam, Optimizer::AdamW, Optimizer::NAdam, Optimizer::RAdam, Optimizer::RmsProp];

    /// Ultralytics' name, also the label.
    pub fn id(self) -> &'static str {
        match self {
            Optimizer::Auto => "auto",
            Optimizer::Sgd => "SGD",
            Optimizer::Adam => "Adam",
            Optimizer::AdamW => "AdamW",
            Optimizer::NAdam => "NAdam",
            Optimizer::RAdam => "RAdam",
            Optimizer::RmsProp => "RMSProp",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TrainingConfig {
    pub epochs: u32,
    pub batch: Batch,
    pub imgsz: u32,
    /// Epochs without improvement before stopping early; 0 = never.
    pub patience: u32,
    pub optimizer: Optimizer,
    /// Initial learning rate.
    pub lr0: f64,
    /// Rectangular batches (less padding); turns off shuffling.
    pub rect: bool,
    /// Varies the image size ±50% between batches.
    pub multi_scale: bool,
    /// Treats every class as one.
    pub single_cls: bool,
    /// Share of the training images used.
    pub fraction: f64,
    /// First N layers kept frozen.
    pub freeze: Option<u32>,
    /// Classification only.
    pub dropout: f64,
    /// Validate during training.
    pub val: bool,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        TrainingConfig {
            epochs: 100,
            batch: Batch::Size(16),
            imgsz: 640,
            patience: 100,
            optimizer: Optimizer::Auto,
            lr0: 0.01,
            rect: false,
            multi_scale: false,
            single_cls: false,
            fraction: 1.0,
            freeze: None,
            dropout: 0.0,
            val: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScheduleConfig {
    /// Final learning rate, as a share of lr0.
    pub lrf: f64,
    pub cos_lr: bool,
    pub momentum: f64,
    pub weight_decay: f64,
    pub warmup_epochs: f64,
    pub warmup_momentum: f64,
    pub warmup_bias_lr: f64,
    /// Nominal batch size the loss is normalized to.
    pub nbs: u32,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        ScheduleConfig {
            lrf: 0.01,
            cos_lr: false,
            momentum: 0.937,
            weight_decay: 0.0005,
            warmup_epochs: 3.0,
            warmup_momentum: 0.8,
            warmup_bias_lr: 0.1,
            nbs: 64,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoAugment {
    #[default]
    RandAugment,
    AutoAugment,
    AugMix,
    Off,
}

impl AutoAugment {
    pub const ALL: [AutoAugment; 4] = [AutoAugment::RandAugment, AutoAugment::AutoAugment, AutoAugment::AugMix, AutoAugment::Off];

    pub fn label(self) -> &'static str {
        match self {
            AutoAugment::RandAugment => "RandAugment",
            AutoAugment::AutoAugment => "AutoAugment",
            AutoAugment::AugMix => "AugMix",
            AutoAugment::Off => "Off",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AugmentationConfig {
    pub hsv_h: f64,
    pub hsv_s: f64,
    pub hsv_v: f64,
    pub degrees: f64,
    pub translate: f64,
    pub scale: f64,
    pub shear: f64,
    pub perspective: f64,
    pub flipud: f64,
    pub fliplr: f64,
    pub bgr: f64,
    pub mosaic: f64,
    /// Mosaic is turned off for the last N epochs.
    pub close_mosaic: u32,
    pub mixup: f64,
    pub cutmix: f64,
    /// Segmentation only.
    pub copy_paste: f64,
    /// Classification only.
    pub erasing: f64,
    /// Classification only.
    pub auto_augment: AutoAugment,
}

impl Default for AugmentationConfig {
    fn default() -> Self {
        AugmentationConfig {
            hsv_h: 0.015,
            hsv_s: 0.7,
            hsv_v: 0.4,
            degrees: 0.0,
            translate: 0.1,
            scale: 0.5,
            shear: 0.0,
            perspective: 0.0,
            flipud: 0.0,
            fliplr: 0.5,
            bgr: 0.0,
            mosaic: 1.0,
            close_mosaic: 10,
            mixup: 0.0,
            cutmix: 0.0,
            copy_paste: 0.0,
            erasing: 0.4,
            auto_augment: AutoAugment::RandAugment,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LossConfig {
    /// Ultralytics' `box` (a Rust keyword).
    #[serde(rename = "box")]
    pub box_gain: f64,
    pub cls: f64,
    pub dfl: f64,
    /// Pose only.
    pub pose: f64,
    /// Pose only: keypoint objectness.
    pub kobj: f64,
    /// Segmentation only.
    pub overlap_mask: bool,
    /// Segmentation only: mask downsample ratio.
    pub mask_ratio: u32,
}

impl Default for LossConfig {
    fn default() -> Self {
        LossConfig { box_gain: 7.5, cls: 0.5, dfl: 1.5, pose: 12.0, kobj: 1.0, overlap_mask: true, mask_ratio: 4 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cache {
    #[default]
    Off,
    Ram,
    Disk,
}

impl Cache {
    pub const ALL: [Cache; 3] = [Cache::Off, Cache::Ram, Cache::Disk];

    pub fn label(self) -> &'static str {
        match self {
            Cache::Off => "Off",
            Cache::Ram => "RAM",
            Cache::Disk => "Disk",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    /// "auto", "cpu", "mps", or GPU ids like "0" or "0,1".
    pub device: String,
    /// Data loading workers per GPU.
    pub workers: u32,
    /// Mixed precision.
    pub amp: bool,
    pub cache: Cache,
    pub seed: u32,
    pub deterministic: bool,
    /// Save a checkpoint every N epochs; `None` = only the last and best.
    pub save_period: Option<u32>,
    /// Stop after this many hours (overrides epochs).
    pub time: Option<f64>,
    pub plots: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        RunConfig {
            device: "auto".into(),
            workers: 8,
            amp: true,
            cache: Cache::Off,
            seed: 0,
            deterministic: true,
            save_period: None,
            time: None,
            plots: true,
        }
    }
}

/// `device` values Ultralytics accepts: auto, cpu, mps or GPU ids.
pub fn valid_device(device: &str) -> bool {
    matches!(device, "auto" | "cpu" | "mps")
        || (!device.is_empty() && device.split(',').all(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit())))
}

impl UltralyticsParams {
    /// The `model` argument: a pretrained file ("yolo11n-seg.pt"), an
    /// architecture to train from scratch ("yolov8s.yaml"), or the chosen file.
    pub fn model_name(&self) -> String {
        let m = &self.model;
        let ext = match &m.weights {
            Weights::File(path) => return path.clone(),
            Weights::Pretrained => "pt",
            Weights::FromScratch => "yaml",
        };
        match m.family {
            Family::RtDetr => format!("rtdetr-{}.{ext}", m.size.id()),
            Family::Yolo11 => format!("yolo11{}{}.{ext}", m.size.id(), m.task.suffix()),
            Family::Yolov8 => format!("yolov8{}{}.{ext}", m.size.id(), m.task.suffix()),
            Family::Yolo12 => format!("yolo12{}{}.{ext}", m.size.id(), m.task.suffix()),
        }
    }

    pub fn summary(&self) -> String {
        let m = &self.model;
        let model = match &m.weights {
            Weights::File(path) => path.rsplit(['/', '\\']).next().unwrap_or(path).to_string(),
            _ if m.family == Family::RtDetr => format!("RT-DETR-{}", m.size.id()),
            _ => format!("{}{}", m.family.label(), m.size.id()),
        };
        format!("{model} · {}", m.task.id())
    }

    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        let (m, t, s, a, l, r) = (&self.model, &self.training, &self.schedule, &self.augmentation, &self.loss, &self.run);
        let mut range = |name: &str, value: f64, lo: f64, hi: f64| {
            if !(lo..=hi).contains(&value) {
                p.push(format!("{name} must be between {lo} and {hi}."));
            }
        };

        range("Final learning rate (lrf)", s.lrf, 0.0, 1.0);
        range("Momentum", s.momentum, 0.0, 1.0);
        range("Weight decay", s.weight_decay, 0.0, 0.1);
        range("Warmup epochs", s.warmup_epochs, 0.0, 1000.0);
        range("Warmup momentum", s.warmup_momentum, 0.0, 1.0);
        range("Warmup bias learning rate", s.warmup_bias_lr, 0.0, 1.0);
        range("Fraction", t.fraction, 0.01, 1.0);
        range("Dropout", t.dropout, 0.0, 0.99);
        for (name, value) in [
            ("Hue shift", a.hsv_h),
            ("Saturation shift", a.hsv_s),
            ("Brightness shift", a.hsv_v),
            ("Translate", a.translate),
            ("Vertical flip", a.flipud),
            ("Horizontal flip", a.fliplr),
            ("Channel swap (BGR)", a.bgr),
            ("Mosaic", a.mosaic),
            ("Mixup", a.mixup),
            ("CutMix", a.cutmix),
            ("Copy-paste", a.copy_paste),
            ("Random erasing", a.erasing),
        ] {
            range(name, value, 0.0, 1.0);
        }
        range("Rotation", a.degrees, 0.0, 180.0);
        range("Shear", a.shear, 0.0, 180.0);
        range("Scale", a.scale, 0.0, 1.0);
        range("Perspective", a.perspective, 0.0, 0.001);
        for (name, value) in [("Box loss gain", l.box_gain), ("Class loss gain", l.cls), ("DFL loss gain", l.dfl), ("Pose loss gain", l.pose), ("Keypoint objectness gain", l.kobj)] {
            range(name, value, 0.0, 100.0);
        }

        if t.epochs == 0 {
            p.push("Train for at least 1 epoch.".into());
        }
        if t.imgsz < 32 || t.imgsz > 4096 || t.imgsz % 32 != 0 {
            p.push("Image size must be a multiple of 32, from 32 to 4096.".into());
        }
        if t.batch.size() == Some(0) {
            p.push("Batch size must be at least 1, or Auto.".into());
        }
        if !(t.lr0 > 0.0 && t.lr0 <= 1.0) {
            p.push("Learning rate must be above 0 and at most 1.".into());
        }
        if s.nbs == 0 {
            p.push("Nominal batch size must be at least 1.".into());
        }
        if l.mask_ratio == 0 {
            p.push("Mask ratio must be at least 1.".into());
        }
        if r.time.is_some_and(|h| h <= 0.0) {
            p.push("Time limit must be above 0 hours.".into());
        }
        if !valid_device(&r.device) {
            p.push("Device must be auto, cpu, mps, or GPU numbers like 0 or 0,1.".into());
        }

        match &m.weights {
            Weights::File(path) => {
                let lower = path.to_lowercase();
                if path.trim().is_empty() {
                    p.push("Choose a weights file.".into());
                } else if !(lower.ends_with(".pt") || lower.ends_with(".yaml") || lower.ends_with(".yml")) {
                    p.push("The weights file must be a .pt or .yaml file.".into());
                }
            }
            weights => {
                if !m.family.sizes().contains(&m.size) {
                    p.push(format!("{} doesn't come in size {}.", m.family.label(), m.size.label()));
                }
                if !m.family.tasks().contains(&m.task) {
                    p.push(format!("{} only does detection.", m.family.label()));
                } else if *weights == Weights::Pretrained && !m.family.pretrained_tasks().contains(&m.task) {
                    p.push(format!(
                        "{} has pretrained weights for detection only; train {} from scratch or pick another model.",
                        m.family.label(),
                        m.task.label().to_lowercase()
                    ));
                }
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ultra(set: &mut ParameterSet) -> &mut UltralyticsParams {
        match &mut set.params {
            FrameworkParams::Ultralytics(p) => p,
        }
    }

    #[test]
    fn round_trips_as_toml() {
        let mut set = ParameterSet::new("baseline");
        set.description = Some("first try".into());
        set.created = "2026-10-01T10:00:00Z".into();
        let p = ultra(&mut set);
        p.model.task = Task::Segment;
        p.model.weights = Weights::File("training/trains/run/best.pt".into());
        p.training.batch = Batch::Auto;
        p.training.freeze = Some(10);
        p.run.time = Some(1.5);

        let text = toml::to_string(&set).unwrap();
        assert!(text.contains("framework = \"ultralytics\""), "{text}");
        assert!(text.contains("[training]"), "{text}");
        assert_eq!(toml::from_str::<ParameterSet>(&text).unwrap(), set);

        let defaults = ParameterSet::new("plain");
        assert_eq!(toml::from_str::<ParameterSet>(&toml::to_string(&defaults).unwrap()).unwrap(), defaults);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let text = "name = \"old\"\nframework = \"ultralytics\"\n[training]\nepochs = 5\n";
        let set: ParameterSet = toml::from_str(text).unwrap();
        let FrameworkParams::Ultralytics(p) = &set.params;
        assert_eq!(p.training.epochs, 5);
        assert_eq!(p.training.imgsz, 640);
        assert_eq!(p.augmentation, AugmentationConfig::default());
        assert_eq!(p.model.weights, Weights::Pretrained);
    }

    #[test]
    fn names_the_model() {
        let mut p = UltralyticsParams::default();
        assert_eq!(p.model_name(), "yolo11n.pt");
        p.model.task = Task::Segment;
        assert_eq!(p.model_name(), "yolo11n-seg.pt");
        p.model.family = Family::Yolov8;
        p.model.size = ModelSize::S;
        p.model.weights = Weights::FromScratch;
        assert_eq!(p.model_name(), "yolov8s-seg.yaml");
        p.model = ModelConfig { task: Task::Detect, family: Family::RtDetr, size: ModelSize::L, weights: Weights::Pretrained };
        assert_eq!(p.model_name(), "rtdetr-l.pt");
        assert_eq!(p.summary(), "RT-DETR-l · detect");
        p.model.weights = Weights::File("C:/runs/best.pt".into());
        assert_eq!(p.model_name(), "C:/runs/best.pt");
        assert_eq!(p.summary(), "best.pt · detect");
    }

    #[test]
    fn finds_problems() {
        let set = ParameterSet::new("ok");
        assert!(set.problems().is_empty(), "{:?}", set.problems());

        let mut bad = ParameterSet::new("bad/name");
        let p = ultra(&mut bad);
        p.training.imgsz = 650;
        p.training.epochs = 0;
        p.augmentation.fliplr = 1.5;
        p.run.device = "gpu0".into();
        p.model.family = Family::RtDetr; // size n, which RT-DETR doesn't have
        let problems = bad.problems();
        assert_eq!(problems.len(), 6, "{problems:?}");

        let mut seg = ParameterSet::new("seg");
        ultra(&mut seg).model = ModelConfig { task: Task::Segment, family: Family::Yolo12, ..Default::default() };
        assert_eq!(seg.problems().len(), 1, "YOLO12 pretrained is detect only");
        ultra(&mut seg).model.weights = Weights::FromScratch;
        assert!(seg.problems().is_empty());
    }

    #[test]
    fn checks_devices() {
        for ok in ["auto", "cpu", "mps", "0", "0,1"] {
            assert!(valid_device(ok), "{ok}");
        }
        for bad in ["", "gpu", "0,", "cuda:0"] {
            assert!(!valid_device(bad), "{bad}");
        }
    }

    #[test]
    fn duplicates_and_compares() {
        let mut set = ParameterSet::new("a");
        set.created = "x".into();
        let copy = set.duplicate();
        assert_eq!(copy.name, "a-copy");
        assert!(copy.created.is_empty());
        let mut touched = set.clone();
        touched.updated = "later".into();
        assert!(set.same_settings(&touched));
        assert!(!set.same_settings(&copy));
    }
}
