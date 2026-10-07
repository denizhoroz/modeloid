//! Tuning hyperparameters with Optuna: which settings can be searched and
//! over what (`HYPERPARAMETERS`, `TuneParam`), what a tuning run is asked to
//! do (`TunePlan`), and the record it keeps in
//! `training/tuning/<name>/tune.toml` (`TuneRecord`). The best trial's values
//! are written into a parameter set.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::parameters::{Batch, Optimizer, ParameterSet, Task, UltralyticsParams};
use crate::train::TrainStatus;

/// A sampled or chosen value: a number, or a name (e.g. an optimizer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TuneValue {
    Number(f64),
    Text(String),
}

impl TuneValue {
    pub fn number(&self) -> Option<f64> {
        match self {
            TuneValue::Number(n) => Some(*n),
            TuneValue::Text(_) => None,
        }
    }

    /// Short text: whole numbers without decimals, others to 6 digits.
    pub fn label(&self) -> String {
        match self {
            TuneValue::Text(t) => t.clone(),
            TuneValue::Number(n) if n.fract() == 0.0 && n.abs() < 1e12 => format!("{n:.0}"),
            // About 4 significant digits (the stored value keeps them all).
            TuneValue::Number(n) => {
                let decimals = (3 - n.abs().log10().floor() as i32).clamp(0, 12) as usize;
                let text = format!("{n:.decimals$}");
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            }
        }
    }
}

/// Which tasks a hyperparameter means something for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Applies {
    All,
    /// Box losses, mosaic...: not for classification.
    NotClassify,
    Only(Task),
}

impl Applies {
    pub fn to(self, task: Task) -> bool {
        match self {
            Applies::All => true,
            Applies::NotClassify => task != Task::Classify,
            Applies::Only(t) => t == task,
        }
    }
}

/// Where a hyperparameter's search starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefaultSpace {
    /// From `low` to `high`; `true` = on a log scale.
    Range(f64, f64, bool),
    Numbers(&'static [f64]),
    Texts(&'static [&'static str]),
}

/// A setting tuning can search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HyperParam {
    /// Ultralytics' argument name, e.g. "lr0".
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub tip: &'static str,
    /// Whole numbers only.
    pub integer: bool,
    pub default: DefaultSpace,
    /// Searched unless switched off.
    pub on: bool,
    pub applies: Applies,
}

const fn hp(
    key: &'static str,
    label: &'static str,
    group: &'static str,
    tip: &'static str,
    default: DefaultSpace,
    on: bool,
    applies: Applies,
) -> HyperParam {
    HyperParam { key, label, group, tip, integer: false, default, on, applies }
}

const fn int(mut p: HyperParam) -> HyperParam {
    p.integer = true;
    p
}

use DefaultSpace::{Numbers, Range, Texts};

/// Every searchable setting, with Ultralytics' tuner ranges as defaults.
pub const HYPERPARAMETERS: [HyperParam; 31] = [
    hp("lr0", "Learning rate", "Training", "Initial learning rate.", Range(1e-5, 1e-1, true), true, Applies::All),
    hp("optimizer", "Optimizer", "Training", "Which optimizer trains the model.", Texts(&["SGD", "AdamW", "Adam"]), false, Applies::All),
    int(hp("batch", "Batch size", "Training", "Images per step; larger needs more GPU memory.", Numbers(&[8.0, 16.0, 32.0]), false, Applies::All)),
    int(hp("imgsz", "Image size", "Training", "Pixels on the long side; a multiple of 32.", Numbers(&[480.0, 640.0]), false, Applies::All)),
    hp("dropout", "Dropout", "Training", "Against overfitting. Classification only.", Range(0.0, 0.5, false), false, Applies::Only(Task::Classify)),
    hp("lrf", "Final learning rate", "Schedule", "Learning rate at the end, as a share of the initial one.", Range(0.01, 1.0, false), true, Applies::All),
    hp("momentum", "Momentum", "Schedule", "SGD momentum, or Adam's beta1.", Range(0.6, 0.98, false), true, Applies::All),
    hp("weight_decay", "Weight decay", "Schedule", "L2 penalty on weights.", Range(0.0, 0.001, false), true, Applies::All),
    hp("warmup_epochs", "Warmup epochs", "Schedule", "Epochs where the learning rate ramps up.", Range(0.0, 5.0, false), true, Applies::All),
    hp("warmup_momentum", "Warmup momentum", "Schedule", "Momentum at the start of warmup.", Range(0.0, 0.95, false), false, Applies::All),
    hp("warmup_bias_lr", "Warmup bias LR", "Schedule", "Learning rate of bias parameters during warmup.", Range(0.0, 0.2, false), false, Applies::All),
    hp("box", "Box loss weight", "Loss", "Weight of the box position loss.", Range(1.0, 20.0, false), true, Applies::NotClassify),
    hp("cls", "Class loss weight", "Loss", "Weight of the classification loss.", Range(0.2, 4.0, false), true, Applies::NotClassify),
    hp("dfl", "DFL loss weight", "Loss", "Weight of the distribution focal loss.", Range(0.4, 6.0, false), false, Applies::NotClassify),
    hp("hsv_h", "Hue shift", "Augmentation", "Random hue change.", Range(0.0, 0.1, false), true, Applies::All),
    hp("hsv_s", "Saturation shift", "Augmentation", "Random saturation change.", Range(0.0, 0.9, false), true, Applies::All),
    hp("hsv_v", "Brightness shift", "Augmentation", "Random brightness change.", Range(0.0, 0.9, false), true, Applies::All),
    hp("degrees", "Rotation (°)", "Augmentation", "Random rotation either way.", Range(0.0, 45.0, false), false, Applies::All),
    hp("translate", "Translate", "Augmentation", "Random shift, as a share of the image.", Range(0.0, 0.9, false), true, Applies::All),
    hp("scale", "Scale", "Augmentation", "Random zoom in or out.", Range(0.0, 0.95, false), true, Applies::All),
    hp("shear", "Shear (°)", "Augmentation", "Random shear.", Range(0.0, 10.0, false), false, Applies::All),
    hp("perspective", "Perspective", "Augmentation", "Random perspective warp.", Range(0.0, 0.001, false), false, Applies::All),
    hp("flipud", "Vertical flip", "Augmentation", "Chance of flipping upside down.", Range(0.0, 1.0, false), false, Applies::All),
    hp("fliplr", "Horizontal flip", "Augmentation", "Chance of a left–right flip.", Range(0.0, 1.0, false), true, Applies::All),
    hp("bgr", "Channel swap", "Augmentation", "Chance of swapping RGB to BGR.", Range(0.0, 1.0, false), false, Applies::All),
    hp("mosaic", "Mosaic", "Augmentation", "Chance of combining 4 images into one.", Range(0.0, 1.0, false), true, Applies::NotClassify),
    hp("mixup", "Mixup", "Augmentation", "Chance of blending two images.", Range(0.0, 1.0, false), false, Applies::NotClassify),
    hp("cutmix", "CutMix", "Augmentation", "Chance of pasting a patch of another image.", Range(0.0, 1.0, false), false, Applies::NotClassify),
    hp("copy_paste", "Copy-paste", "Augmentation", "Chance of copying objects between images. Segmentation only.", Range(0.0, 1.0, false), false, Applies::Only(Task::Segment)),
    int(hp("close_mosaic", "Close mosaic", "Augmentation", "Mosaic off for the last N epochs.", Range(0.0, 20.0, false), false, Applies::NotClassify)),
    hp("erasing", "Random erasing", "Augmentation", "Chance of erasing a patch. Classification only.", Range(0.0, 0.9, false), false, Applies::Only(Task::Classify)),
];

pub fn hyperparameter(key: &str) -> Option<&'static HyperParam> {
    HYPERPARAMETERS.iter().find(|h| h.key == key)
}

/// How one hyperparameter is searched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TuneSpace {
    /// Any value from `low` to `high` (log scale: evenly over orders of magnitude).
    Range { low: f64, high: f64, log: bool },
    /// Only these values.
    Candidates(Vec<TuneValue>),
}

/// A row of the search space table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TuneParam {
    pub key: String,
    /// Searched; otherwise the parameter set's value stays.
    pub enabled: bool,
    pub space: TuneSpace,
}

impl TuneParam {
    /// The default row for `h`.
    pub fn default_for(h: &HyperParam) -> TuneParam {
        let space = match h.default {
            Range(low, high, log) => TuneSpace::Range { low, high, log },
            Numbers(values) => TuneSpace::Candidates(values.iter().map(|v| TuneValue::Number(*v)).collect()),
            Texts(values) => TuneSpace::Candidates(values.iter().map(|v| TuneValue::Text(v.to_string())).collect()),
        };
        TuneParam { key: h.key.to_string(), enabled: h.on, space }
    }

    /// What's wrong with this row, if it is searched.
    pub fn problem(&self) -> Option<String> {
        let h = hyperparameter(&self.key)?;
        if !self.enabled {
            return None;
        }
        match &self.space {
            TuneSpace::Range { .. } if matches!(h.default, Texts(_)) => Some(format!("{}: choose candidates, it has no range.", h.label)),
            TuneSpace::Range { low, high, .. } if !(low.is_finite() && high.is_finite() && low < high) => {
                Some(format!("{}: the lowest value must be below the highest.", h.label))
            }
            TuneSpace::Range { low, log: true, .. } if *low <= 0.0 => Some(format!("{}: a log scale needs a lowest value above 0.", h.label)),
            TuneSpace::Range { low, high, .. } if h.integer && high.floor() - low.ceil() < 1.0 => {
                Some(format!("{}: the range holds fewer than two whole numbers.", h.label))
            }
            TuneSpace::Range { .. } => None,
            TuneSpace::Candidates(values) if values.is_empty() => Some(format!("{}: add at least one candidate.", h.label)),
            TuneSpace::Candidates(values) => match h.default {
                Texts(allowed) => values
                    .iter()
                    .find(|v| !matches!(v, TuneValue::Text(t) if allowed.contains(&t.as_str()) || is_optimizer(h.key, t)))
                    .map(|v| format!("{}: \"{}\" isn't one of its choices.", h.label, v.label())),
                _ => values
                    .iter()
                    .find(|v| v.number().is_none_or(|n| !n.is_finite() || n < 0.0 || (h.integer && n.fract() != 0.0)))
                    .map(|v| format!("{}: \"{}\" isn't a {}.", h.label, v.label(), if h.integer { "whole number" } else { "number" })),
            },
        }
    }
}

fn is_optimizer(key: &str, name: &str) -> bool {
    key == "optimizer" && Optimizer::ALL.iter().any(|o| o.id() == name)
}

/// The default search space for `task`: every hyperparameter that applies to it.
#[cfg(test)]
fn default_space(task: Task) -> Vec<TuneParam> {
    HYPERPARAMETERS.iter().filter(|h| h.applies.to(task)).map(TuneParam::default_for).collect()
}

/// `key`'s value in `p`, if `key` is a hyperparameter.
pub fn hyper_value(p: &UltralyticsParams, key: &str) -> Option<TuneValue> {
    let (t, s, a, l) = (&p.training, &p.schedule, &p.augmentation, &p.loss);
    let n = |v: f64| Some(TuneValue::Number(v));
    match key {
        "lr0" => n(t.lr0),
        "optimizer" => Some(TuneValue::Text(t.optimizer.id().to_string())),
        "batch" => match t.batch {
            Batch::Auto => Some(TuneValue::Text("auto".into())),
            Batch::Size(b) => n(f64::from(b)),
        },
        "imgsz" => n(f64::from(t.imgsz)),
        "dropout" => n(t.dropout),
        "lrf" => n(s.lrf),
        "momentum" => n(s.momentum),
        "weight_decay" => n(s.weight_decay),
        "warmup_epochs" => n(s.warmup_epochs),
        "warmup_momentum" => n(s.warmup_momentum),
        "warmup_bias_lr" => n(s.warmup_bias_lr),
        "box" => n(l.box_gain),
        "cls" => n(l.cls),
        "dfl" => n(l.dfl),
        "hsv_h" => n(a.hsv_h),
        "hsv_s" => n(a.hsv_s),
        "hsv_v" => n(a.hsv_v),
        "degrees" => n(a.degrees),
        "translate" => n(a.translate),
        "scale" => n(a.scale),
        "shear" => n(a.shear),
        "perspective" => n(a.perspective),
        "flipud" => n(a.flipud),
        "fliplr" => n(a.fliplr),
        "bgr" => n(a.bgr),
        "mosaic" => n(a.mosaic),
        "mixup" => n(a.mixup),
        "cutmix" => n(a.cutmix),
        "copy_paste" => n(a.copy_paste),
        "close_mosaic" => n(f64::from(a.close_mosaic)),
        "erasing" => n(a.erasing),
        _ => None,
    }
}

/// Writes `value` into `p` as hyperparameter `key`.
pub fn set_hyper_value(p: &mut UltralyticsParams, key: &str, value: &TuneValue) -> Result<(), String> {
    if key == "optimizer" {
        let TuneValue::Text(name) = value else { return Err("optimizer: not a name".into()) };
        p.training.optimizer = Optimizer::ALL.iter().copied().find(|o| o.id() == name).ok_or_else(|| format!("unknown optimizer {name}"))?;
        return Ok(());
    }
    let v = value.number().ok_or_else(|| format!("{key}: not a number"))?;
    let whole = v.round().max(0.0) as u32;
    let (t, s, a, l) = (&mut p.training, &mut p.schedule, &mut p.augmentation, &mut p.loss);
    match key {
        "lr0" => t.lr0 = v,
        "batch" => t.batch = Batch::Size(whole.max(1)),
        "imgsz" => t.imgsz = whole,
        "dropout" => t.dropout = v,
        "lrf" => s.lrf = v,
        "momentum" => s.momentum = v,
        "weight_decay" => s.weight_decay = v,
        "warmup_epochs" => s.warmup_epochs = v,
        "warmup_momentum" => s.warmup_momentum = v,
        "warmup_bias_lr" => s.warmup_bias_lr = v,
        "box" => l.box_gain = v,
        "cls" => l.cls = v,
        "dfl" => l.dfl = v,
        "hsv_h" => a.hsv_h = v,
        "hsv_s" => a.hsv_s = v,
        "hsv_v" => a.hsv_v = v,
        "degrees" => a.degrees = v,
        "translate" => a.translate = v,
        "scale" => a.scale = v,
        "shear" => a.shear = v,
        "perspective" => a.perspective = v,
        "flipud" => a.flipud = v,
        "fliplr" => a.fliplr = v,
        "bgr" => a.bgr = v,
        "mosaic" => a.mosaic = v,
        "mixup" => a.mixup = v,
        "cutmix" => a.cutmix = v,
        "copy_paste" => a.copy_paste = v,
        "close_mosaic" => a.close_mosaic = whole,
        "erasing" => a.erasing = v,
        _ => return Err(format!("{key} isn't a hyperparameter")),
    }
    Ok(())
}

/// What a tuning run is asked to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TunePlan {
    pub dataset: u32,
    /// The parameter set to start from and to save the best values into; `None` = `new_set`.
    pub parameters: Option<String>,
    /// A new parameter set (name, model and other settings) saved with the best values.
    pub new_set: Option<ParameterSet>,
    /// Name of the tuning run (`training/tuning/<name>`).
    pub name: String,
    pub trials: u32,
    /// Epochs each trial trains.
    pub epochs: u32,
    /// Share of the training images each trial uses (faster trials).
    pub fraction: f64,
    pub space: Vec<TuneParam>,
}

pub const MAX_TRIALS: u32 = 500;
pub const MAX_TRIAL_EPOCHS: u32 = 300;

impl TunePlan {
    /// Problems with the plan itself (the dataset and model are checked by the core).
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !(1..=MAX_TRIALS).contains(&self.trials) {
            problems.push(format!("Trials: 1 to {MAX_TRIALS}."));
        }
        if !(1..=MAX_TRIAL_EPOCHS).contains(&self.epochs) {
            problems.push(format!("Epochs per trial: 1 to {MAX_TRIAL_EPOCHS}."));
        }
        if !(self.fraction > 0.0 && self.fraction <= 1.0) {
            problems.push("Data per trial: more than 0% and at most 100%.".into());
        }
        if !self.space.iter().any(|p| p.enabled) {
            problems.push("Choose at least one hyperparameter to tune.".into());
        }
        problems.extend(self.space.iter().filter_map(TuneParam::problem));
        match (&self.parameters, &self.new_set) {
            (None, None) => problems.push("Choose a parameter set, or create a new one.".into()),
            (None, Some(set)) if set.name.trim().is_empty() => problems.push("The new parameter set needs a name.".into()),
            _ => {}
        }
        problems
    }

    /// The searched rows that apply to `task`.
    pub fn searched(&self, task: Task) -> Vec<&TuneParam> {
        self.space.iter().filter(|p| p.enabled && hyperparameter(&p.key).is_some_and(|h| h.applies.to(task))).collect()
    }
}

/// One trial's outcome.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrialResult {
    /// From 1.
    pub number: u32,
    /// Ultralytics' fitness (higher is better); `None` for a failed trial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    pub failed: bool,
    #[serde(default)]
    pub params: BTreeMap<String, TuneValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `tune.toml` in a tuning run's folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TuneRecord {
    pub name: String,
    pub dataset_id: u32,
    pub dataset_name: String,
    /// The parameter set the best values go into.
    pub parameters: String,
    /// `parameters` is created by this run (else it existed and is updated).
    pub new_set: bool,
    pub model: String,
    pub task: String,
    pub device: String,
    pub trials: u32,
    pub epochs: u32,
    pub fraction: f64,
    /// Searched hyperparameter keys.
    #[serde(default)]
    pub searched: Vec<String>,
    pub started: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished: Option<String>,
    #[serde(default)]
    pub status: TrainStatus,
    #[serde(default)]
    pub results: Vec<TrialResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub best: Option<TrialResult>,
    /// The parameter set the best values were saved into.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn tune_name_taken_message(name: &str) -> String {
    format!("A tuning run named '{name}' already exists in training/tuning. Choose another name.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_space_follows_the_task() {
        let detect = default_space(Task::Detect);
        assert!(detect.iter().any(|p| p.key == "box"));
        assert!(!detect.iter().any(|p| p.key == "dropout" || p.key == "copy_paste"));
        let classify = default_space(Task::Classify);
        assert!(classify.iter().any(|p| p.key == "dropout"));
        assert!(!classify.iter().any(|p| p.key == "mosaic"));
        assert!(detect.iter().all(|p| p.problem().is_none()), "defaults are valid");
    }

    #[test]
    fn checks_rows() {
        let mut p = TuneParam::default_for(hyperparameter("lr0").unwrap());
        p.space = TuneSpace::Range { low: 0.0, high: 0.1, log: true };
        assert!(p.problem().unwrap().contains("log scale"));
        p.space = TuneSpace::Range { low: 0.2, high: 0.1, log: false };
        assert!(p.problem().is_some());
        p.enabled = false;
        assert!(p.problem().is_none(), "off rows aren't checked");

        let mut opt = TuneParam::default_for(hyperparameter("optimizer").unwrap());
        opt.space = TuneSpace::Candidates(vec![TuneValue::Text("RMSProp".into())]);
        opt.enabled = true;
        assert!(opt.problem().is_none(), "any Ultralytics optimizer");
        opt.space = TuneSpace::Candidates(vec![TuneValue::Text("Lion".into())]);
        assert!(opt.problem().is_some());

        let mut batch = TuneParam::default_for(hyperparameter("batch").unwrap());
        batch.enabled = true;
        batch.space = TuneSpace::Candidates(vec![TuneValue::Number(12.5)]);
        assert!(batch.problem().unwrap().contains("whole number"));
    }

    #[test]
    fn reads_and_writes_every_hyperparameter() {
        let mut p = UltralyticsParams::default();
        for h in HYPERPARAMETERS {
            let value = match h.default {
                Texts(t) => TuneValue::Text(t[1].to_string()),
                Numbers(n) => TuneValue::Number(n[0]),
                Range(_, high, _) => TuneValue::Number(if h.integer { high } else { high / 2.0 }),
            };
            set_hyper_value(&mut p, h.key, &value).unwrap();
            assert_eq!(hyper_value(&p, h.key), Some(value), "{}", h.key);
        }
        assert_eq!(TuneValue::Number(0.00123).label(), "0.00123");
        assert_eq!(TuneValue::Number(13.271988).label(), "13.27");
        assert_eq!(TuneValue::Number(0.0000192373).label(), "0.00001924");
        assert_eq!(TuneValue::Number(640.0).label(), "640");
    }

    #[test]
    fn plan_needs_a_target_and_a_search() {
        let plan = TunePlan {
            dataset: 1,
            parameters: None,
            new_set: None,
            name: "t".into(),
            trials: 10,
            epochs: 5,
            fraction: 0.5,
            space: default_space(Task::Detect),
        };
        assert_eq!(plan.problems(), ["Choose a parameter set, or create a new one."]);
        let none = TunePlan { parameters: Some("base".into()), space: vec![], ..plan };
        assert_eq!(none.problems(), ["Choose at least one hyperparameter to tune."]);
    }
}
