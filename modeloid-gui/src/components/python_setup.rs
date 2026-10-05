//! The Python environment training runs in.
//!
//! - On launch, `PythonAutoSetup` (in `AppLayout`) sets the environment up
//!   when it needs it: Python, Ultralytics, Optuna and the CPU build of
//!   PyTorch. These are defaults that can't be removed.
//! - Settings → Python environment (`PythonSetupCard`) chooses the CPU or GPU,
//!   and installs or removes the GPU build of PyTorch (only torch and
//!   torchvision change).
//! - Before training, `LibrariesMissing` reminds the user and links there.

use dioxus::prelude::*;

use crate::api::error_message;
use crate::api::jobs::JobKind;
use crate::api::pyenv::{
    prepare_python_env, python_env_overview, set_train_device, start_python_setup, EnvOverview, EnvStatus, TorchVariant, TrainDevice,
};
use crate::components::{set_if_newer, use_job, JobProgress};
use crate::routes::Route;

/// Sizes on Windows with Python 3.12, measured with the CUDA 13.0 torch build
/// (uv's download cache is cleared after installing).
const GPU_TEXT: &str = "Training runs on CUDA supported GPU. Faster than CPU.";
const GPU_DOWNLOAD_NOTE: &str = "Training runs on CUDA supported GPU. Faster than CPU. About 2.2 GB to download, around 3.5 GB on disk.";

fn status_text(status: &EnvStatus) -> (&'static str, &'static str) {
    match status {
        EnvStatus::Missing => ("Not set up", "is-missing"),
        EnvStatus::NoLibraries(_) => ("Not finished", "is-error"),
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

/// Status, and a card each for the GPU and the CPU to choose which one
/// training runs on. The CPU's libraries are always there; the GPU's build of
/// PyTorch is installed or removed here (removing switches back to the CPU
/// build). The CUDA build runs on the CPU too.
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
    let usable = ov.status.usable();
    let device = ov.device();

    let run = move |task: Task| async move {
        error.set(None);
        confirm_remove.set(false);
        let started = match task {
            Task::Prepare => prepare_python_env().await.map(|job| job.into_iter().collect::<Vec<_>>()),
            Task::Install(variant) => start_python_setup(variant).await.map(|job| vec![job]),
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
            h3 { class: "python-setup-title", "Training Device" }
            div { class: "python-status",
                span { class: "python-status-badge {status_class}", "{status_label}" }
                match &ov.status {
                    EnvStatus::Ready(_) | EnvStatus::Outdated(_) => rsx! {},
                    EnvStatus::NoLibraries(_) => rsx! {
                        span { class: "python-status-detail",
                            if running { "Installing the training libraries (PyTorch, Ultralytics, Optuna)…" } else { "The training libraries (PyTorch, Ultralytics, Optuna) didn't finish installing." }
                        }
                    },
                    EnvStatus::Broken(reason) => rsx! { span { class: "python-status-detail", "{reason}" } },
                    EnvStatus::Missing => rsx! {
                        span { class: "python-status-detail",
                            if running { "Setting up Python and the training libraries…" } else { "Modeloid keeps its own Python; nothing else on this computer changes." }
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
                EnvStatus::Broken(_) | EnvStatus::NoLibraries(_) => rsx! {
                    div { class: "form-actions",
                        button { class: "button", r#type: "button", disabled: running, onclick: move |_| run(Task::Prepare), "Repair" }
                    }
                },
                EnvStatus::Outdated(_) => rsx! {
                    div { class: "notice notice-warning python-notice",
                        span { "This version of Modeloid needs newer packages." }
                        button { class: "button button-sm", r#type: "button", disabled: running, onclick: move |_| run(Task::Prepare), "Update" }
                    }
                },
                _ => rsx! {},
            }

            if usable {
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
                                TrainDevice::Gpu if installed => GPU_TEXT.to_string(),
                                TrainDevice::Gpu => GPU_DOWNLOAD_NOTE.to_string(),
                                TrainDevice::Cpu => "Training runs on CPU. Slower than GPU.".to_string(),
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
                                    // The CPU's libraries are defaults: nothing to install or remove.
                                    if option == TrainDevice::Gpu {
                                        div { class: "device-actions",
                                            button {
                                                class: if installed { "button button-secondary button-sm" } else { "button button-sm" },
                                                r#type: "button",
                                                disabled: installed || !available || running,
                                                onclick: move |_| run(Task::Install(TorchVariant::Cuda)),
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
            }

            if confirm_remove() {
                div { class: "notice notice-warning param-confirm", role: "alert",
                    span { "Remove the GPU build of PyTorch? Modeloid switches back to the CPU build (about 0.3 GB to download); everything else stays." }
                    button { class: "button button-danger button-sm", r#type: "button", onclick: move |_| run(Task::Install(TorchVariant::Cpu)), "Remove" }
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
    /// Set up, repair or update the environment (the default libraries).
    Prepare,
    /// Switch to this PyTorch build (only torch and torchvision change).
    Install(TorchVariant),
}

/// Sets the environment up on launch when it needs it (Python and the default libraries).
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

/// The reminder shown when a training is about to start before the libraries
/// are installed: what's missing and a way to Settings, where setup shows. Render it instead of starting the training when
/// `python_env_overview()` says `!status.usable()`.
#[component]
pub fn LibrariesMissing(on_close: EventHandler<()>) -> Element {
    rsx! {
        div { class: "notice notice-warning python-notice", role: "alert",
            span {
                strong { "The training libraries aren't installed yet. " }
                "Modeloid installs PyTorch, Ultralytics and Optuna when it sets up Python. Check Settings → Python environment, then start the training again."
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
