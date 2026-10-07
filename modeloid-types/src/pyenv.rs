//! The Python environment training runs in: what's installed and on what
//! hardware. Shared so the GUI's browser side can show it.

use serde::{Deserialize, Serialize};

/// Which PyTorch build is installed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TorchVariant {
    /// CPU only (on macOS: the standard build, which also uses Apple's GPU).
    #[default]
    Cpu,
    /// NVIDIA GPUs, CUDA 13.0.
    Cuda,
}

impl TorchVariant {
    pub fn label(self) -> &'static str {
        match self {
            TorchVariant::Cpu => "CPU",
            TorchVariant::Cuda => "GPU (CUDA)",
        }
    }

    /// The device this build is meant for.
    pub fn device(self) -> TrainDevice {
        match self {
            TorchVariant::Cpu => TrainDevice::Cpu,
            TorchVariant::Cuda => TrainDevice::Gpu,
        }
    }

    /// The pyservice extra that installs it.
    pub fn extra(self) -> &'static str {
        match self {
            TorchVariant::Cpu => "cpu",
            TorchVariant::Cuda => "cu130",
        }
    }
}

/// What training runs on. The CUDA build of PyTorch runs on the CPU too, so
/// choosing the CPU never needs a reinstall; choosing the GPU needs the CUDA build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrainDevice {
    Gpu,
    Cpu,
}

impl TrainDevice {
    pub fn label(self) -> &'static str {
        match self {
            TrainDevice::Gpu => "GPU (CUDA)",
            TrainDevice::Cpu => "CPU",
        }
    }

    /// The PyTorch build this device needs.
    pub fn variant(self) -> TorchVariant {
        match self {
            TrainDevice::Gpu => TorchVariant::Cuda,
            TrainDevice::Cpu => TorchVariant::Cpu,
        }
    }

    /// Ultralytics' `device` for a parameter set whose device is "auto".
    pub fn ultralytics_device(self) -> &'static str {
        match self {
            TrainDevice::Gpu => "0",
            TrainDevice::Cpu => "cpu",
        }
    }
}

/// An NVIDIA GPU found with `nvidia-smi`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gpu {
    pub name: String,
    pub memory_mb: u32,
    pub driver: String,
    /// CUDA compute capability, e.g. "8.6"; older `nvidia-smi` versions don't report it.
    pub compute_cap: Option<String>,
}

/// Oldest driver that runs the CUDA 13 build.
pub const MIN_CUDA_DRIVER: u32 = 580;
/// Oldest GPU generation CUDA 13 supports (Turing, RTX 20-series), as compute capability ×10.
pub const MIN_COMPUTE_CAP: u32 = 75;

impl Gpu {
    /// Whether the driver is new enough for the CUDA build.
    pub fn driver_supports_cuda(&self) -> bool {
        self.driver.split('.').next().and_then(|major| major.parse::<u32>().ok()).is_some_and(|major| major >= MIN_CUDA_DRIVER)
    }

    /// Whether the GPU itself is new enough (unknown counts as yes; the driver check still applies).
    pub fn gpu_supports_cuda(&self) -> bool {
        let Some(cap) = &self.compute_cap else { return true };
        let (major, minor) = cap.split_once('.').unwrap_or((cap, "0"));
        match (major.parse::<u32>(), minor.parse::<u32>()) {
            (Ok(major), Ok(minor)) => major * 10 + minor >= MIN_COMPUTE_CAP,
            _ => true,
        }
    }

    /// Why the CUDA build won't work here, if it won't.
    pub fn cuda_problem(&self) -> Option<String> {
        if !self.gpu_supports_cuda() {
            Some(format!("{} is too old for the CUDA build (it needs an RTX 20-series or newer).", self.name))
        } else if !self.driver_supports_cuda() {
            Some(format!("The NVIDIA driver ({}) is too old for the CUDA build; update it to {MIN_CUDA_DRIVER} or newer.", self.driver))
        } else {
            None
        }
    }

    /// Parses one line of `nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv,noheader`,
    /// e.g. "NVIDIA GeForce RTX 3060, 12288 MiB, 617.14, 8.6". The compute capability may be missing.
    pub fn parse_smi_line(line: &str) -> Option<Gpu> {
        let mut parts = line.split(',').map(str::trim);
        let name = parts.next().filter(|n| !n.is_empty())?.to_string();
        let memory_mb = parts.next()?.trim_end_matches("MiB").trim().parse().ok()?;
        let driver = parts.next().filter(|d| !d.is_empty())?.to_string();
        let compute_cap = parts.next().filter(|c| c.contains('.')).map(str::to_string);
        Some(Gpu { name, memory_mb, driver, compute_cap })
    }
}

/// What a working environment has, as reported by `main.py check`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvInfo {
    pub variant: TorchVariant,
    pub python: String,
    pub torch: String,
    pub torchvision: String,
    pub ultralytics: String,
    pub cuda_available: bool,
    pub mps_available: bool,
    pub device_name: String,
    /// Hash of the `uv.lock` it was installed from; a different one means the
    /// app's dependencies changed since.
    pub lock_hash: String,
    /// Optuna's version (hyperparameter tuning).
    #[serde(default)]
    pub optuna: String,
    /// Albumentations' version (Augmentation); empty in an environment
    /// installed before it was added.
    #[serde(default)]
    pub albumentations: String,
    /// huggingface_hub's version (Hugging Face downloads); empty in an
    /// environment installed before it was added.
    #[serde(default)]
    pub huggingface_hub: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnvStatus {
    /// Not created yet (the app sets it up on its first launch).
    Missing,
    /// Python is there (this version), but not the libraries (PyTorch,
    /// Ultralytics, Optuna): their install didn't finish; setup runs it again.
    NoLibraries(String),
    Ready(EnvInfo),
    /// Works, but was installed from an older `uv.lock`.
    Outdated(EnvInfo),
    /// Half installed or damaged; Repair sets it up again.
    Broken(String),
    /// Modeloid's folder can't be written to (e.g. installed in Program Files).
    NotWritable(String),
}

impl EnvStatus {
    pub fn info(&self) -> Option<&EnvInfo> {
        match self {
            EnvStatus::Ready(info) | EnvStatus::Outdated(info) => Some(info),
            _ => None,
        }
    }

    /// Training can run (an outdated environment still works).
    pub fn usable(&self) -> bool {
        self.info().is_some()
    }
}

/// Everything the setup screen needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvOverview {
    pub status: EnvStatus,
    pub gpu: Option<Gpu>,
    /// The folder holding .venv, python/ and src/pyservice.
    pub root: String,
    /// The device the user chose in Settings; `None` = not chosen yet.
    pub device_choice: Option<TrainDevice>,
}

impl EnvOverview {
    /// Whether this device can train with what's installed: the CPU with any
    /// build, the GPU only with the CUDA build.
    pub fn installed(&self, device: TrainDevice) -> bool {
        match (self.status.info(), device) {
            (None, _) => false,
            (Some(_), TrainDevice::Cpu) => true,
            (Some(info), TrainDevice::Gpu) => info.variant == TorchVariant::Cuda,
        }
    }

    /// The GPU can be offered: an NVIDIA GPU new enough for the CUDA build.
    pub fn gpu_usable(&self) -> bool {
        self.gpu.as_ref().is_some_and(|g| g.cuda_problem().is_none())
    }

    /// What training runs on: the user's choice, else the GPU when the CUDA
    /// build is installed (or, before setup, when there's a usable GPU).
    pub fn device(&self) -> TrainDevice {
        match self.device_choice {
            Some(TrainDevice::Gpu) if !self.gpu_usable() => TrainDevice::Cpu,
            Some(device) => device,
            None if self.status.info().is_some() => {
                if self.installed(TrainDevice::Gpu) { TrainDevice::Gpu } else { TrainDevice::Cpu }
            }
            None => self.suggested_variant().device(),
        }
    }

    /// The build to suggest: CUDA when there's an NVIDIA GPU with a new enough driver.
    pub fn suggested_variant(&self) -> TorchVariant {
        match &self.gpu {
            Some(gpu) if gpu.cuda_problem().is_none() => TorchVariant::Cuda,
            _ => TorchVariant::Cpu,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi() {
        let gpu = Gpu::parse_smi_line("NVIDIA GeForce RTX 3060, 12288 MiB, 617.14, 8.6").unwrap();
        assert_eq!(
            gpu,
            Gpu { name: "NVIDIA GeForce RTX 3060".into(), memory_mb: 12288, driver: "617.14".into(), compute_cap: Some("8.6".into()) }
        );
        assert_eq!(gpu.cuda_problem(), None);
        assert!(Gpu { driver: "560.94".into(), ..gpu.clone() }.cuda_problem().is_some(), "old driver");
        assert!(Gpu { compute_cap: Some("6.1".into()), ..gpu }.cuda_problem().is_some(), "GTX 10-series");
        assert_eq!(Gpu::parse_smi_line("Old smi, 4096 MiB, 590.10").unwrap().compute_cap, None);
        assert_eq!(Gpu::parse_smi_line(""), None);
        assert_eq!(Gpu::parse_smi_line("No devices were found"), None);
    }

    #[test]
    fn suggests_cuda_only_with_a_capable_gpu() {
        let mut overview = EnvOverview { status: EnvStatus::Missing, gpu: None, root: String::new(), device_choice: None };
        assert_eq!(overview.suggested_variant(), TorchVariant::Cpu);
        overview.gpu = Gpu::parse_smi_line("RTX, 8192 MiB, 590.10, 7.5");
        assert_eq!(overview.suggested_variant(), TorchVariant::Cuda);
        overview.gpu = Gpu::parse_smi_line("Old, 4096 MiB, 590.10, 6.1");
        assert_eq!(overview.suggested_variant(), TorchVariant::Cpu);
    }

    #[test]
    fn cpu_runs_on_any_build_gpu_needs_cuda() {
        let cuda = EnvInfo { variant: TorchVariant::Cuda, ..Default::default() };
        let mut overview = EnvOverview {
            status: EnvStatus::Ready(cuda),
            gpu: Gpu::parse_smi_line("RTX, 8192 MiB, 590.10, 8.6"),
            root: String::new(),
            device_choice: None,
        };
        assert!(overview.installed(TrainDevice::Gpu) && overview.installed(TrainDevice::Cpu));
        assert_eq!(overview.device(), TrainDevice::Gpu);
        overview.device_choice = Some(TrainDevice::Cpu);
        assert_eq!(overview.device(), TrainDevice::Cpu, "no reinstall needed");

        overview.status = EnvStatus::Ready(EnvInfo::default()); // CPU build
        assert!(!overview.installed(TrainDevice::Gpu));
        overview.device_choice = None;
        assert_eq!(overview.device(), TrainDevice::Cpu);

        overview.gpu = None;
        overview.device_choice = Some(TrainDevice::Gpu);
        assert_eq!(overview.device(), TrainDevice::Cpu, "no GPU to train on");

        overview.status = EnvStatus::NoLibraries("3.12.13".into());
        assert!(!overview.status.usable());
        assert!(!overview.installed(TrainDevice::Cpu), "Python alone can't train");
    }
}
