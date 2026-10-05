//! The Python environment training runs in. Everything lives under one root
//! folder (`<repo>/modeloid` in development, `<install>/modeloid` when
//! installed), so removing Modeloid removes it all:
//!
//! ```text
//! .venv/                    the environment (UV_PROJECT_ENVIRONMENT)
//! .venv/modeloid-python.txt Python version, written when the environment is created
//! .venv/modeloid-env.json   the training libraries installed (EnvInfo)
//! python/                   uv-managed CPython (UV_PYTHON_INSTALL_DIR)
//! .uv-cache/                uv's download cache (UV_CACHE_DIR)
//! bin/uv(.exe)              uv itself, bundled or downloaded
//! weights/                  pretrained weights Ultralytics downloads (shared by every run)
//! .ultralytics/             Ultralytics' settings and fonts (YOLO_CONFIG_DIR)
//! src/pyservice/            pyproject.toml, uv.lock, main.py
//! ```
//!
//! Setup never touches a Python the user installed: uv only uses its own
//! managed Python in `python/`.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, fs, thread};

use crate::core::progress::{Cancelled, Progress, Reporter, Unit};

pub use modeloid_types::pyenv::*;

/// Python version the pyservice project pins (`requires-python`).
pub const PYTHON_VERSION: &str = "3.12";
/// uv release downloaded when neither `bin/uv` nor a `uv` on PATH exists.
pub const UV_VERSION: &str = "0.11.26";
/// Overrides where the root folder is.
pub const HOME_ENV: &str = "MODELOID_HOME";
const ENV_FILE: &str = "modeloid-env.json";
const PYTHON_FILE: &str = "modeloid-python.txt";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PyEnv {
    root: PathBuf,
}

impl PyEnv {
    /// An environment rooted at `root` (the folder holding `src/pyservice`).
    pub fn at(root: impl Into<PathBuf>) -> Self {
        PyEnv { root: root.into() }
    }

    /// Finds the root: `MODELOID_HOME`, then `modeloid/` next to the
    /// executable (installed), then the `modeloid` crate folder (development).
    pub fn locate() -> Result<Self, crate::Error> {
        let has_service = |dir: &Path| dir.join("src").join("pyservice").join("pyproject.toml").is_file();
        if let Some(home) = env::var_os(HOME_ENV).filter(|v| !v.is_empty()) {
            let root = PathBuf::from(home);
            return if has_service(&root) {
                Ok(PyEnv::at(root))
            } else {
                Err(format!("{HOME_ENV} is set to {}, but src/pyservice/pyproject.toml isn't there", root.display()).into())
            };
        }
        let installed = env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("modeloid")));
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        installed
            .into_iter()
            .chain([dev])
            .find(|dir| has_service(dir))
            .map(PyEnv::at)
            .ok_or_else(|| "Modeloid's Python files (modeloid/src/pyservice) weren't found next to the app".into())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn venv(&self) -> PathBuf {
        self.root.join(".venv")
    }

    /// The environment's Python executable.
    pub fn python(&self) -> PathBuf {
        if cfg!(windows) { self.venv().join("Scripts").join("python.exe") } else { self.venv().join("bin").join("python") }
    }

    pub fn pyservice(&self) -> PathBuf {
        self.root.join("src").join("pyservice")
    }

    pub fn python_dir(&self) -> PathBuf {
        self.root.join("python")
    }

    /// Pretrained weights Ultralytics downloads; Python runs from here so they land in it.
    pub fn weights_dir(&self) -> PathBuf {
        self.root.join("weights")
    }

    /// Ultralytics' own settings folder, kept out of the user's profile.
    pub fn ultralytics_dir(&self) -> PathBuf {
        self.root.join(".ultralytics")
    }

    /// `python main.py <args>` in the environment, set up for training:
    /// unbuffered UTF-8 output, Ultralytics' folders under the root.
    pub fn service_command<I, S>(&self, args: I) -> Result<Command, crate::Error>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        fs::create_dir_all(self.weights_dir())?;
        fs::create_dir_all(self.ultralytics_dir())?;
        let mut cmd = Command::new(self.python());
        cmd.arg(self.pyservice().join("main.py"))
            .args(args)
            .current_dir(self.weights_dir())
            .env("PYTHONUNBUFFERED", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("YOLO_CONFIG_DIR", self.ultralytics_dir());
        Ok(cmd)
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join(".uv-cache")
    }

    fn bundled_uv(&self) -> PathBuf {
        self.root.join("bin").join(if cfg!(windows) { "uv.exe" } else { "uv" })
    }

    fn env_file(&self) -> PathBuf {
        self.venv().join(ENV_FILE)
    }

    fn python_file(&self) -> PathBuf {
        self.venv().join(PYTHON_FILE)
    }

    /// Hash of `uv.lock`, to notice when the app's dependencies changed.
    pub fn lock_hash(&self) -> String {
        fs::read(self.pyservice().join("uv.lock")).map(|bytes| format!("{:016x}", fnv1a(&bytes))).unwrap_or_default()
    }

    /// Whether files can be created in the root folder.
    fn writable(&self) -> bool {
        let probe = self.root.join(".modeloid-write-test");
        let ok = fs::write(&probe, b"").is_ok();
        let _ = fs::remove_file(&probe);
        ok
    }

    pub fn status(&self) -> EnvStatus {
        if !self.venv().exists() {
            return if self.writable() { EnvStatus::Missing } else { EnvStatus::NotWritable(self.root.display().to_string()) };
        }
        if !self.python().is_file() {
            return EnvStatus::Broken("The environment's Python is missing.".into());
        }
        match fs::read_to_string(self.env_file()) {
            // No libraries yet, or their install didn't finish (installing again repairs it).
            Err(_) => EnvStatus::NoLibraries(fs::read_to_string(self.python_file()).unwrap_or_default().trim().to_string()),
            Ok(text) => match serde_json::from_str::<EnvInfo>(&text) {
                Ok(info) if info.lock_hash != self.lock_hash() => EnvStatus::Outdated(info),
                Ok(info) => EnvStatus::Ready(info),
                Err(e) => EnvStatus::Broken(format!("{ENV_FILE} is damaged: {e}")),
            },
        }
    }

    /// Everything the setup screen shows, with the user's choices from the settings.
    pub fn overview(&self, settings: &crate::Settings) -> EnvOverview {
        EnvOverview {
            status: self.status(),
            gpu: detect_gpu(),
            root: self.root.display().to_string(),
            device_choice: settings.train_device(),
        }
    }

    /// The uv to run: `bin/uv`, then a `uv` on PATH, else `None` (download it with `fetch_uv`).
    pub fn find_uv(&self) -> Option<PathBuf> {
        let bundled = self.bundled_uv();
        if bundled.is_file() {
            return Some(bundled);
        }
        let mut probe = Command::new("uv");
        probe.arg("--version").stdout(Stdio::null()).stderr(Stdio::null());
        hide_window(&mut probe);
        probe.status().ok().filter(|s| s.success()).map(|_| PathBuf::from("uv"))
    }

    /// Downloads uv `UV_VERSION` for this computer into `bin/`.
    pub async fn fetch_uv(&self, reporter: &dyn Reporter) -> Result<PathBuf, crate::Error> {
        let asset = uv_asset().ok_or("There's no uv download for this kind of computer")?;
        let url = format!("https://github.com/astral-sh/uv/releases/download/{UV_VERSION}/{asset}");
        let bin = self.root.join("bin");
        let tmp = bin.join("download");
        tokio::fs::create_dir_all(&tmp).await?;
        let archive = tmp.join(asset);
        let client = reqwest::Client::builder().user_agent("modeloid").build()?;
        let progress = reporter.start("Downloading uv", Unit::Bytes);
        let downloaded = crate::core::io::download_into(&client, &url, &archive, progress.as_ref()).await;
        progress.finish();
        downloaded?;

        let unpacked = tmp.join("unpacked");
        if asset.ends_with(".zip") {
            crate::core::io::extract_zip(&archive, &unpacked, reporter).await?;
        } else {
            let (archive, unpacked) = (archive.clone(), unpacked.clone());
            tokio::task::spawn_blocking(move || extract_tar_gz(&archive, &unpacked)).await??;
        }
        let name = if cfg!(windows) { "uv.exe" } else { "uv" };
        let found = walk_find(&unpacked, name).ok_or("The uv download didn't contain uv")?;
        let dest = self.bundled_uv();
        tokio::fs::copy(&found, &dest).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).await?;
        }
        let _ = tokio::fs::remove_dir_all(&tmp).await;
        Ok(dest)
    }

    /// A uv command with the environment's folders set, so uv only touches them.
    fn uv(&self, uv: &Path) -> Command {
        let mut cmd = Command::new(uv);
        cmd.env("UV_PROJECT_ENVIRONMENT", self.venv())
            .env("UV_PYTHON_INSTALL_DIR", self.python_dir())
            .env("UV_CACHE_DIR", self.cache_dir())
            .env("UV_PYTHON_PREFERENCE", "only-managed")
            .env("UV_NO_PROGRESS", "1")
            .env("NO_COLOR", "1")
            .current_dir(self.pyservice());
        cmd
    }

    /// Creates the environment with only Python in it (about 70 MB). Blocking;
    /// stops when cancelled. `install` calls it when needed.
    pub fn create(&self, uv: &Path, reporter: &dyn Reporter) -> Result<String, crate::Error> {
        let step = reporter.start("Installing Python", Unit::Steps);
        let mut cmd = self.uv(uv);
        cmd.args(["python", "install", PYTHON_VERSION]);
        run(cmd, step.as_ref(), reporter)?;
        step.finish();

        let step = reporter.start("Creating the environment", Unit::Steps);
        let mut cmd = self.uv(uv);
        cmd.args(["venv", "--allow-existing", "--python", PYTHON_VERSION]).arg(self.venv());
        run(cmd, step.as_ref(), reporter)?;
        let mut version = Command::new(self.python());
        version.args(["-c", "import platform; print(platform.python_version())"]);
        let version = run(version, step.as_ref(), reporter)?.trim().to_string();
        fs::write(self.python_file(), &version)?;
        step.finish();
        Ok(version)
    }

    /// Installs everything from `uv.lock` (Ultralytics, Optuna...) with
    /// `variant`'s PyTorch build, creating the environment first if needed,
    /// then checks the result. Switching builds only swaps torch and
    /// torchvision; nothing else is ever removed. Blocking; stops (killing uv)
    /// when cancelled. Until it finishes the status is `NoLibraries`, and
    /// running it again repairs.
    pub fn install(&self, variant: TorchVariant, uv: &Path, reporter: &dyn Reporter) -> Result<EnvInfo, crate::Error> {
        let _ = fs::remove_file(self.env_file());
        if !self.python().is_file() || !self.python_file().is_file() {
            self.create(uv, reporter)?;
        }

        let step = reporter.start("Installing libraries", Unit::Steps);
        let mut cmd = self.uv(uv);
        cmd.args(["sync", "--locked", "--no-dev", "--extra", variant.extra()]);
        run(cmd, step.as_ref(), reporter)?;
        step.finish();

        let step = reporter.start("Checking", Unit::Steps);
        let info = self.check(variant, step.as_ref(), reporter)?;
        step.finish();
        fs::write(self.env_file(), serde_json::to_vec_pretty(&info)?)?;

        // The environment's files are hardlinks into (or copies of) the
        // cache, so it can go: that drops PyTorch builds left from earlier
        // installs, and file managers stop counting the libraries twice.
        let step = reporter.start("Cleaning up", Unit::Steps);
        let mut cmd = self.uv(uv);
        cmd.args(["cache", "clean"]);
        if let Err(e) = run(cmd, step.as_ref(), reporter) {
            reporter.message(&format!("Couldn't clear uv's download cache: {e}"));
        }
        step.finish();
        Ok(info)
    }

    /// What setup has to do, if anything: the PyTorch build to (re)install.
    /// A new or half-installed environment gets the CPU build; an outdated
    /// one is updated with the build it has.
    pub fn pending_install(&self) -> Option<TorchVariant> {
        match self.status() {
            EnvStatus::Ready(_) | EnvStatus::NotWritable(_) => None,
            EnvStatus::Outdated(info) => Some(info.variant),
            EnvStatus::Missing | EnvStatus::NoLibraries(_) | EnvStatus::Broken(_) => Some(TorchVariant::Cpu),
        }
    }

    /// Runs `main.py check` in the environment.
    fn check(&self, variant: TorchVariant, progress: &dyn Progress, reporter: &dyn Reporter) -> Result<EnvInfo, crate::Error> {
        #[derive(serde::Deserialize)]
        struct Checked {
            error: Option<String>,
            #[serde(default)]
            python: String,
            #[serde(default)]
            torch: String,
            #[serde(default)]
            torchvision: String,
            #[serde(default)]
            ultralytics: String,
            #[serde(default)]
            cuda_available: bool,
            #[serde(default)]
            mps_available: bool,
            #[serde(default)]
            device_name: String,
            #[serde(default)]
            optuna: String,
            #[serde(default)]
            albumentations: String,
        }
        let mut cmd = Command::new(self.python());
        cmd.arg("main.py").arg("check").current_dir(self.pyservice()).env("PYTHONIOENCODING", "utf-8");
        let output = run(cmd, progress, reporter)?;
        let line = output.lines().rev().find(|l| l.trim_start().starts_with('{')).ok_or("The environment check printed nothing")?;
        let checked: Checked = serde_json::from_str(line)?;
        if let Some(error) = checked.error {
            return Err(format!("The environment check failed: {error}").into());
        }
        Ok(EnvInfo {
            variant,
            python: checked.python,
            torch: checked.torch,
            torchvision: checked.torchvision,
            ultralytics: checked.ultralytics,
            cuda_available: checked.cuda_available,
            mps_available: checked.mps_available,
            device_name: checked.device_name,
            lock_hash: self.lock_hash(),
            optuna: checked.optuna,
            albumentations: checked.albumentations,
        })
    }
}

/// The NVIDIA GPU, if `nvidia-smi` finds one.
pub fn detect_gpu() -> Option<Gpu> {
    let mut cmd = Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,memory.total,driver_version,compute_cap", "--format=csv,noheader"]);
    hide_window(&mut cmd);
    let output = cmd.output().ok().filter(|o| o.status.success())?;
    String::from_utf8_lossy(&output.stdout).lines().find_map(Gpu::parse_smi_line)
}

/// Runs `cmd`, passing each stderr line to `reporter` as a message and
/// returning stdout. Kills it when `progress` is cancelled.
pub fn run(mut cmd: Command, progress: &dyn Progress, reporter: &dyn Reporter) -> Result<String, crate::Error> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_window(&mut cmd);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start {program}: {e}"))?;

    let mut stdout = child.stdout.take().expect("piped");
    let out_reader = thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let stderr = BufReader::new(child.stderr.take().expect("piped"));
    let err_lines = lines.clone();
    let err_reader = thread::spawn(move || {
        for line in stderr.lines().map_while(Result::ok) {
            err_lines.lock().unwrap().push(line);
        }
    });

    let mut shown = 0;
    let status = loop {
        {
            let lines = lines.lock().unwrap();
            for line in lines[shown..].iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
                reporter.message(line);
            }
            shown = lines.len();
        }
        if progress.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    let _ = err_reader.join();
    let stdout = out_reader.join().unwrap_or_default();
    if status.success() {
        Ok(stdout)
    } else {
        // uv puts the reason in its last lines.
        let lines = lines.lock().unwrap();
        let tail: Vec<&str> = lines.iter().rev().filter(|l| !l.trim().is_empty()).take(3).map(String::as_str).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        Err(format!("{program} failed ({status}): {}", tail.join(" ")).into())
    }
}

/// One piece of a child process's output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// A finished line (ended with a newline).
    Line(String),
    /// A line redrawn in place (ended with a carriage return), e.g. a progress bar.
    Progress(String),
}

/// Runs `cmd`, handing stdout and stderr to `on_output` as they come, split
/// into finished lines and in-place progress updates. Kills it when
/// `progress` is cancelled. Errors when it exits with a failure.
pub fn stream(mut cmd: Command, progress: &dyn Progress, mut on_output: impl FnMut(Output)) -> Result<(), crate::Error> {
    use std::sync::mpsc;

    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_window(&mut cmd);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("couldn't start {program}: {e}"))?;

    let (tx, rx) = mpsc::channel::<Output>();
    let readers: Vec<_> = [
        Box::new(child.stdout.take().expect("piped")) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().expect("piped")),
    ]
    .into_iter()
    .map(|mut pipe| {
        let tx = tx.clone();
        thread::spawn(move || {
            let mut splitter = LineSplitter::default();
            let mut buf = [0u8; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for out in splitter.push(&buf[..n]) {
                    let _ = tx.send(out);
                }
            }
            if let Some(out) = splitter.finish() {
                let _ = tx.send(out);
            }
        })
    })
    .collect();
    drop(tx);

    let status = loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(out) => on_output(out),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(status) = child.try_wait()? {
                    break status;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        if progress.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
    };
    for reader in readers {
        let _ = reader.join();
    }
    for out in rx.try_iter() {
        on_output(out);
    }
    if status.success() { Ok(()) } else { Err(format!("{program} failed ({status})").into()) }
}

/// Splits bytes into `Output`s on `\n` (finished) and `\r` (redrawn); `\r\n` is
/// one newline. Lossy UTF-8.
#[derive(Default)]
struct LineSplitter {
    buf: Vec<u8>,
    after_cr: bool,
}

impl LineSplitter {
    fn take(&mut self) -> String {
        String::from_utf8_lossy(&std::mem::take(&mut self.buf)).into_owned()
    }

    fn push(&mut self, bytes: &[u8]) -> Vec<Output> {
        let mut out = Vec::new();
        for &b in bytes {
            if self.after_cr {
                self.after_cr = false;
                if b == b'\n' {
                    out.push(Output::Line(self.take()));
                    continue;
                }
                if !self.buf.is_empty() {
                    out.push(Output::Progress(self.take()));
                }
            }
            match b {
                b'\n' => out.push(Output::Line(self.take())),
                b'\r' => self.after_cr = true,
                _ => self.buf.push(b),
            }
        }
        out
    }

    fn finish(&mut self) -> Option<Output> {
        (!self.buf.is_empty()).then(|| Output::Line(self.take()))
    }
}

/// No console window flashing up when the GUI's server runs tools on Windows.
fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// The uv release file for this OS and CPU.
pub fn uv_asset() -> Option<&'static str> {
    uv_asset_for(env::consts::OS, env::consts::ARCH)
}

fn uv_asset_for(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("windows", "x86_64") => "uv-x86_64-pc-windows-msvc.zip",
        ("windows", "aarch64") => "uv-aarch64-pc-windows-msvc.zip",
        ("linux", "x86_64") => "uv-x86_64-unknown-linux-gnu.tar.gz",
        ("linux", "aarch64") => "uv-aarch64-unknown-linux-gnu.tar.gz",
        ("macos", "x86_64") => "uv-x86_64-apple-darwin.tar.gz",
        ("macos", "aarch64") => "uv-aarch64-apple-darwin.tar.gz",
        _ => return None,
    })
}

#[cfg(not(windows))]
fn extract_tar_gz(archive: &Path, out: &Path) -> Result<(), crate::Error> {
    let file = fs::File::open(archive)?;
    tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(out)?;
    Ok(())
}

#[cfg(windows)]
fn extract_tar_gz(_archive: &Path, _out: &Path) -> Result<(), crate::Error> {
    Err("uv for Windows comes as a zip".into())
}

/// First file named `name` under `dir`.
fn walk_find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = walk_find(&path, name) {
                return Some(found);
            }
        } else if path.file_name().is_some_and(|n| n == name) {
            return Some(path);
        }
    }
    None
}

/// FNV-1a: stable across Rust versions, unlike `DefaultHasher`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let dir = env::temp_dir().join(format!("modeloid-pyenv-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("src").join("pyservice")).unwrap();
            fs::write(dir.join("src").join("pyservice").join("uv.lock"), "lock v1").unwrap();
            TempRoot(dir)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reports_status_from_disk() {
        let tmp = TempRoot::new("status");
        let env = PyEnv::at(&tmp.0);
        assert_eq!(env.status(), EnvStatus::Missing);
        assert_eq!(env.pending_install(), Some(TorchVariant::Cpu), "the CPU build by default");

        fs::create_dir_all(env.venv()).unwrap();
        assert!(matches!(env.status(), EnvStatus::Broken(_)), "venv without its Python");

        fs::create_dir_all(env.python().parent().unwrap()).unwrap();
        fs::write(env.python(), "").unwrap();
        fs::write(env.python_file(), "3.12.13\n").unwrap();
        assert_eq!(env.status(), EnvStatus::NoLibraries("3.12.13".into()));

        let info = EnvInfo { variant: TorchVariant::Cuda, torch: "2.14.1+cu130".into(), lock_hash: env.lock_hash(), ..Default::default() };
        fs::write(env.env_file(), serde_json::to_vec(&info).unwrap()).unwrap();
        assert_eq!(env.status(), EnvStatus::Ready(info.clone()));
        assert_eq!(env.pending_install(), None);

        fs::write(env.pyservice().join("uv.lock"), "lock v2").unwrap();
        assert_eq!(env.status(), EnvStatus::Outdated(info.clone()));
        assert_eq!(env.pending_install(), Some(TorchVariant::Cuda), "updated with the build it has");

        // An install that didn't finish (its record gone): Python stays.
        fs::remove_file(env.env_file()).unwrap();
        assert_eq!(env.status(), EnvStatus::NoLibraries("3.12.13".into()));
    }

    #[test]
    fn splits_lines_and_progress() {
        let mut s = LineSplitter::default();
        let mut out = s.push(b"one\r\ntwo\n\r 10%");
        out.extend(s.push(b"\r 50%\rdone\n"));
        assert_eq!(
            out,
            [
                Output::Line("one".into()),
                Output::Line("two".into()),
                Output::Progress(" 10%".into()),
                Output::Progress(" 50%".into()),
                Output::Line("done".into()),
            ]
        );
        assert_eq!(s.push(b"tail"), []);
        assert_eq!(s.finish(), Some(Output::Line("tail".into())));
    }

    #[test]
    fn hashes_stably() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn picks_uv_downloads() {
        assert_eq!(uv_asset_for("windows", "x86_64"), Some("uv-x86_64-pc-windows-msvc.zip"));
        assert_eq!(uv_asset_for("macos", "aarch64"), Some("uv-aarch64-apple-darwin.tar.gz"));
        assert_eq!(uv_asset_for("freebsd", "x86_64"), None);
    }

    #[test]
    fn locates_the_dev_root() {
        // Without MODELOID_HOME (not set in tests) the crate folder is found.
        if env::var_os(HOME_ENV).is_none() {
            let env = PyEnv::locate().unwrap();
            assert!(env.pyservice().join("pyproject.toml").is_file());
        }
    }
}
