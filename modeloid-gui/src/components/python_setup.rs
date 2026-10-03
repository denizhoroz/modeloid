//! The Python environment training runs in.
//!
//! - On launch, `PythonAutoSetup` (in `AppLayout`) creates the environment
//!   with only Python in it; nothing big is downloaded.
//! - The training libraries (PyTorch for the CPU or GPU, Ultralytics) are
//!   installed and removed from Settings → Python environment
//!   (`PythonSetupCard`). The environment itself is never deleted there.
//! - Before training, `LibrariesMissing` reminds the user and links there.

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::api::pyenv::{
    install_tuning, prepare_python_env, python_env_overview, remove_python_libraries, remove_tuning, set_train_device, start_python_setup,
    EnvOverview, EnvStatus, TorchVariant, TrainDevice,
};
use crate::components::{set_if_newer, use_job, JobProgress};
use crate::routes::Route;

/// Sizes on Windows with Python 3.12: downloads from uv.lock, CPU disk use
/// measured after setup, CUDA disk use estimated from its 2 GB torch wheel.
fn download_note(device: TrainDevice) -> &'static str {
    match device {
        TrainDevice::Gpu => "Much faster training. About 2.2 GB to download, around 5 GB on disk.",
        TrainDevice::Cpu => "About 0.3 GB to download, 1 GB on disk. Training runs much slower.",
    }
}

fn status_text(status: &EnvStatus) -> (&'static str, &'static str) {
    match status {
        EnvStatus::Missing => ("Not set up", "is-missing"),
        EnvStatus::NoLibraries(_) => ("Libraries not installed", "is-error"),
        EnvStatus::Ready(_) => ("Ready", "is-ready"),
        EnvStatus::Outdated(_) => ("Update available", "is-warning"),
        EnvStatus::Broken(_) => ("Needs repair", "is-error"),
        EnvStatus::NotWritable(_) => ("Can't be set up here", "is-error"),
    }
}

/// Opens Settings and scrolls to the Python environment section.
pub fn open_python_settings() {
    navigator().push(Route::Settings {});
    // The page renders after navigating; wait for the section, then scroll.
    document::eval(
        "let n = 0; const t = setInterval(() => { const e = document.getElementById('settings-python'); \
         if (e || ++n > 40) { clearInterval(t); e && e.scrollIntoView({ block: 'start', behavior: 'smooth' }); } }, 50);",
    );
}

/// Status, and a card each for the GPU and the CPU: install or remove its
/// libraries, and choose which one training runs on. The CUDA build runs on
/// the CPU too, so with it installed both cards are installed.
#[component]
pub fn PythonSetupCard() -> Element {
    let (job, _) = use_job(JobKind::PythonSetup);
    let running = job.read().as_ref().is_some_and(|j| !j.finished);
    // Re-check when a step finishes.
    let finished_version = use_memo(move || job.read().as_ref().filter(|j| j.finished).map(|j| j.version));
    let mut overview = use_resource(move || async move {
        let _ = finished_version();
        python_env_overview().await
    });
    let mut error = use_signal(|| None::<String>);
    let mut confirm_remove = use_signal(|| false);

    let current: Option<EnvOverview> = overview.read().as_ref().and_then(|r| r.as_ref().ok()).cloned();
    let Some(ov) = current else {
        return rsx! {
            div { class: "settings-card",
                match &*overview.read() {
                    Some(Err(e)) => rsx! { p { class: "form-error", role: "alert", "{error_message(e)}" } },
                    _ => rsx! { p { class: "field-hint", "Checking the Python environment…" } },
                }
            }
        };
    };

    let (status_label, status_class) = status_text(&ov.status);
    let has_python = ov.status.has_python();
    let device = ov.device();
    let both_installed = ov.installed(TrainDevice::Gpu);

    let run = move |task: Task| async move {
        error.set(None);
        confirm_remove.set(false);
        let started = match task {
            Task::Prepare => prepare_python_env().await.map(|job| job.into_iter().collect::<Vec<_>>()),
            Task::Install(variant) => start_python_setup(variant).await.map(|job| vec![job]),
            Task::Remove => remove_python_libraries().await.map(|job| vec![job]),
            Task::InstallTuning => install_tuning().await.map(|job| vec![job]),
            Task::RemoveTuning => remove_tuning().await.map(|job| vec![job]),
        };
        match started {
            Ok(jobs) => jobs.into_iter().for_each(|started| set_if_newer(job, started)),
            Err(e) => error.set(Some(error_message(&e))),
        }
    };
    let choose = move |device: TrainDevice| async move {
        error.set(None);
        match set_train_device(device).await {
            Ok(()) => overview.restart(),
            Err(e) => error.set(Some(error_message(&e))),
        }
    };

    rsx! {
        div { class: "settings-card python-setup",
            div { class: "python-status",
                span { class: "python-status-badge {status_class}", "{status_label}" }
                match &ov.status {
                    EnvStatus::Ready(info) | EnvStatus::Outdated(info) => rsx! {
                        span { class: "python-status-detail",
                            "Python {info.python} · torch {info.torch} · Ultralytics {info.ultralytics} · trains on {device.label()}"
                        }
                    },
                    EnvStatus::NoLibraries(python) => rsx! {
                        span { class: "python-status-detail",
                            if python.is_empty() { "Python is ready. " } else { "Python {python} is ready. " }
                            "Install the training libraries (PyTorch, Ultralytics) for the GPU or the CPU to train."
                        }
                    },
                    EnvStatus::Broken(reason) => rsx! { span { class: "python-status-detail", "{reason}" } },
                    EnvStatus::Missing => rsx! {
                        span { class: "python-status-detail",
                            if running { "Setting up Python…" } else { "Modeloid keeps its own Python; nothing else on this computer changes." }
                        }
                    },
                    EnvStatus::NotWritable(root) => rsx! {
                        span { class: "python-status-detail", "Modeloid can't write to {root}. Move Modeloid to a folder you own (e.g. in your user folder) and open it from there." }
                    },
                }
            }

            match &ov.status {
                EnvStatus::Missing => rsx! {
                    div { class: "form-actions",
                        button { class: "button", r#type: "button", disabled: running, onclick: move |_| run(Task::Prepare), "Set up Python" }
                    }
                },
                EnvStatus::Broken(_) => rsx! {
                    div { class: "form-actions",
                        button { class: "button", r#type: "button", disabled: running, onclick: move |_| run(Task::Prepare), "Repair" }
                    }
                },
                EnvStatus::Outdated(info) => {
                    let build = info.variant;
                    rsx! {
                        div { class: "notice notice-warning python-notice",
                            span { "This version of Modeloid needs newer packages." }
                            button { class: "button button-sm", r#type: "button", disabled: running, onclick: move |_| run(Task::Install(build)), "Update" }
                        }
                    }
                },
                _ => rsx! {},
            }

            if has_python {
                div { class: "device-list", role: "radiogroup", aria_label: "Train on",
                    for option in [TrainDevice::Gpu, TrainDevice::Cpu] {
                        {
                            let installed = ov.installed(option);
                            let available = option == TrainDevice::Cpu || ov.gpu_usable();
                            let selected = installed && device == option;
                            let detail = match option {
                                TrainDevice::Gpu if !available => match ov.gpu.as_ref().and_then(|g| g.cuda_problem()) {
                                    Some(problem) => format!("No CUDA compatible device found. {problem}"),
                                    None => "No CUDA compatible device found.".to_string(),
                                },
                                TrainDevice::Gpu if installed => format!("{}. Much faster training.", ov.gpu.as_ref().map(|g| g.name.as_str()).unwrap_or("NVIDIA GPU")),
                                TrainDevice::Cpu if both_installed => "Runs on the GPU libraries too; nothing extra installed. Training is much slower.".to_string(),
                                TrainDevice::Cpu if installed => "Training runs much slower than on a GPU.".to_string(),
                                _ => download_note(option).to_string(),
                            };
                            let class = match (selected, available, installed) {
                                (_, false, _) => "device-card is-disabled",
                                (true, _, _) => "device-card is-selected",
                                // Install first; until then the card can't be chosen.
                                (_, _, false) => "device-card is-locked",
                                _ => "device-card",
                            };
                            rsx! {
                                div { key: "{option.label()}", class: "device-option",
                                    // The whole card chooses the device (when its libraries are installed).
                                    label { class,
                                        input {
                                            class: "visually-hidden",
                                            r#type: "radio",
                                            name: "python-device",
                                            checked: selected,
                                            disabled: !installed || running,
                                            aria_describedby: "device-detail-{option.variant().extra()}",
                                            onchange: move |_| choose(option),
                                        }
                                        span { class: "device-name",
                                            "{option.label()}"
                                            if installed {
                                                span { class: "tag tag-ok", "installed" }
                                            } else {
                                                span { class: "tag tag-missing", "not installed" }
                                            }
                                        }
                                        span { id: "device-detail-{option.variant().extra()}", class: "device-detail", "{detail}" }
                                    }
                                    div { class: "device-actions",
                                        button {
                                            class: if installed { "button button-secondary button-sm" } else { "button button-sm" },
                                            r#type: "button",
                                            disabled: installed || !available || running,
                                            onclick: move |_| run(Task::Install(option.variant())),
                                            if installed { "Installed" } else { "Install" }
                                        }
                                        button {
                                            class: "button button-ghost button-sm",
                                            r#type: "button",
                                            disabled: !installed || running,
                                            onclick: move |_| confirm_remove.set(true),
                                            "Remove"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Optional, under the GPU and CPU: Optuna for Tune Hyperparameters.
            if has_python {
                {
                    let info = ov.status.info().cloned();
                    let libraries = ov.status.usable();
                    let tuning = info.as_ref().filter(|i| i.has_tuning()).map(|i| i.optuna.clone());
                    let detail = match (&tuning, libraries) {
                        (Some(version), _) => format!("Optuna {version}. Tune Hyperparameters searches training settings with it."),
                        (None, true) => "Optional. Tune Hyperparameters needs it: Optuna searches training settings for the best ones. About 15 MB.".to_string(),
                        (None, false) => "Optional, for Tune Hyperparameters. Install the GPU or CPU libraries first.".to_string(),
                    };
                    rsx! {
                        div { id: "settings-tuning", class: "device-option tuning-option",
                            div { class: if tuning.is_some() { "device-card is-static is-selected" } else { "device-card is-static" },
                                span { class: "device-name",
                                    "Hyperparameter tuning"
                                    if tuning.is_some() {
                                        span { class: "tag tag-ok", "installed" }
                                    } else {
                                        span { class: "tag tag-missing", "not installed" }
                                    }
                                }
                                span { class: "device-detail", "{detail}" }
                            }
                            div { class: "device-actions",
                                button {
                                    class: if tuning.is_some() { "button button-secondary button-sm" } else { "button button-sm" },
                                    r#type: "button",
                                    disabled: tuning.is_some() || !libraries || running,
                                    onclick: move |_| run(Task::InstallTuning),
                                    if tuning.is_some() { "Installed" } else { "Install" }
                                }
                                button {
                                    class: "button button-ghost button-sm",
                                    r#type: "button",
                                    disabled: tuning.is_none() || running,
                                    onclick: move |_| run(Task::RemoveTuning),
                                    "Remove"
                                }
                            }
                        }
                    }
                }
            }

            if confirm_remove() {
                div { class: "notice notice-warning param-confirm", role: "alert",
                    span {
                        "Remove PyTorch, torchvision and Ultralytics? "
                        if both_installed { "This removes them for both the GPU and the CPU. " }
                        "Python stays; training works again once you install them."
                    }
                    button { class: "button button-danger button-sm", r#type: "button", onclick: move |_| run(Task::Remove), "Remove" }
                    button { class: "button button-secondary button-sm", r#type: "button", onclick: move |_| confirm_remove.set(false), "Cancel" }
                }
            }
            if let Some(message) = error() {
                p { class: "form-error", role: "alert", "{message}" }
            }
            p { class: "field-hint", "Installed in {ov.root}." }

            if let Some(job) = job() {
                JobProgress { job, title: "Python setup", noun: "step", verb: "done", output_label: "Result" }
            }
        }
    }
}

/// What a button in the card starts.
#[derive(Clone, Copy, PartialEq)]
enum Task {
    /// Create (or repair) the environment with Python only.
    Prepare,
    /// Install the training libraries with this build (replacing the other).
    Install(TorchVariant),
    /// Uninstall the training libraries.
    Remove,
    /// Add Optuna (hyperparameter tuning) next to the training libraries.
    InstallTuning,
    RemoveTuning,
}

/// Creates the environment (Python only) on launch when there's none yet.
/// Renders nothing; progress shows in the status bar.
#[component]
pub fn PythonAutoSetup() -> Element {
    use_future(|| async {
        let _ = prepare_python_env().await;
    });
    rsx! {}
}

/// On training pages, while the libraries can't train yet: a note pointing to Settings.
#[component]
pub fn PythonNotice(#[props(into)] text: String) -> Element {
    let overview = use_resource(python_env_overview);
    let usable = matches!(&*overview.read(), Some(Ok(ov)) if ov.status.usable());
    if usable || overview.read().is_none() {
        return rsx! {};
    }
    rsx! {
        div { class: "notice notice-warning python-notice",
            span { "{text}" }
            button { class: "button button-secondary button-sm", r#type: "button", onclick: move |_| open_python_settings(), "Open Python settings" }
        }
    }
}

/// The reminder shown when a training is about to start without the
/// libraries: what's missing and a way to Settings, where the user picks the
/// CPU or GPU to install. Render it instead of starting the training when
/// `python_env_overview()` says `!status.usable()`.
#[component]
pub fn LibrariesMissing(on_close: EventHandler<()>) -> Element {
    rsx! {
        div { class: "notice notice-warning python-notice", role: "alert",
            span {
                strong { "The training libraries aren't installed. " }
                "Training needs PyTorch and Ultralytics. Install them for the GPU or the CPU in Settings → Python environment, then start the training again."
            }
            button {
                class: "button button-sm",
                r#type: "button",
                onclick: move |_| {
                    on_close.call(());
                    open_python_settings();
                },
                "Go to Python settings"
            }
            button { class: "button button-ghost button-sm", r#type: "button", onclick: move |_| on_close.call(()), "Not now" }
        }
    }
}
