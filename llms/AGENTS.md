# AGENTS.md: Modeloid for AI coding agents

This file is the starting point for any AI agent (Claude, Codex, Copilot, Cursor, Gemini, …) working on Modeloid. Read it fully before changing code. It covers what the project is, how it is built, where things live, the rules the code and the UI follow, and the traps already found and solved.

Also read:

- `modeloid-gui/AGENTS.md`: the Dioxus **0.7** API reference. Dioxus 0.7 changed every API; `cx`, `Scope` and `use_state` are gone. Don't write Dioxus code from memory of older versions.
- The doc comment at the top of `modeloid/src/pyservice/main.py`: the protocol between Rust and Python.
- `modeloid-gui/assets/main.css` (tokens at the top) and `modeloid-gui/assets/themes.css` (the colour token contract at the top) before touching styles.

---

# 1. What Modeloid is

Modeloid is a **local desktop-style tool for computer vision ML work**: datasets, training, tuning and results, all kept in a **workspace folder on disk**. It runs on the user's own computer as a small local web server plus a browser UI. Nothing loads from the internet at runtime (fonts, icons and CSS are bundled); the only network use is what the user asks for: dataset downloads (Roboflow, Kaggle, Hugging Face, direct zip links), the Python environment setup and pretrained weights.

What the user can do:

- **Workspaces**: create / load a workspace folder; options: description, seed, default label format.
- **Dataset Manager** (sidebar group): Datasets (Create, Edit, Download, Import), Labelling (box editor), Explore (statistics + gallery), Health Check (find and fix problems), Classes & Merge (class mapping, merging datasets, dataset links), Split & Balance, Convert (formats, outlines → boxes, classification crops), Augmentation (Albumentations).
- **Training** (sidebar group): Parameters (training parameter sets), Train Model (Ultralytics YOLO / RT-DETR), Performance (a run's metrics, charts and plots), Tune Hyperparameters (Optuna).
- **History** of every job and action; **Settings**: appearance (16 themes, 4 fonts, text size), system limits, cache, Python environment / training device (CPU or NVIDIA CUDA), API keys.

Not started yet: model evaluations (`evals/` folder exists), release packaging / installer.

Platforms: Windows, macOS, Linux. Only Windows gets release downloads (`release.yml`); Linux and macOS users build with `installer/package.sh` (README "Building from source"). The main developer works on Windows; keep everything cross-platform (paths, processes, file locking, dialogs).

---

# 2. Tech stack

| Part | Technology |
| --- | --- |
| Language | Rust (edition 2024 for `modeloid` and `modeloid-types`, 2021 for `modeloid-gui`); recent stable toolchain |
| UI | [Dioxus 0.7](https://dioxuslabs.com/learn/0.7) fullstack, **web platform**: a local axum server + a WASM browser app |
| CLI for the UI | `dx` (dioxus-cli 0.7.x) |
| Training / ML | Python 3.12 service (`modeloid/src/pyservice/main.py`): Ultralytics, PyTorch, Optuna, Albumentations, huggingface_hub |
| Python env | [uv](https://github.com/astral-sh/uv), fully managed by the app (own CPython, own venv); never the user's Python |
| Storage | Plain files: TOML (`workspace.toml`, `dataset.toml`, parameter sets, run records), JSON caches, JSONL history |
| Secrets | OS credential store via `keyring` (Windows Credential Manager / macOS Keychain / Secret Service) |
| HTTP client | `reqwest` 0.12 with rustls/ring, **the same version dioxus-fullstack uses** (one HTTP stack in the binary) |

---

# 3. Repository layout

```
modeloid/                       repo root (Cargo workspace)
├─ Cargo.toml                   workspace + release size profiles (release, server-release, wasm-release)
├─ modeloid-types/              plain shared types + input rules (serde only, builds for WASM)
├─ modeloid/                    the core library: all logic, all disk / network / process work
│  └─ src/
│     ├─ lib.rs                 Error/Result types, public re-exports
│     ├─ models/                Workspace, Dataset, ParameterSet, Settings (TOML-backed)
│     ├─ core/                  one module per feature (see §5)
│     └─ pyservice/             Python side: main.py, pyproject.toml, uv.lock (committed)
├─ modeloid-gui/                the Dioxus app
│  ├─ AGENTS.md                 Dioxus 0.7 API reference
│  ├─ Dioxus.toml, clippy.toml
│  ├─ assets/                   main.css (tokens + rules), themes.css (colour tokens), fonts/, favicon
│  └─ src/
│     ├─ main.rs                launch, server router, request guard
│     ├─ launcher.rs            (server) single instance, free port, opens the browser
│     ├─ main_thread.rs         (server) native dialogs on the main thread (macOS)
│     ├─ routes.rs              the Route enum
│     ├─ nav.rs                 sidebar structure (NAV); tool pages are looked up here
│     ├─ state.rs               context state (workspace, selections, activity long-poll)
│     ├─ util.rs                formatting helpers (format_count, plural, …)
│     ├─ api/                   server functions: thin layer over the core
│     ├─ components/            shared UI components (table, charts, sliders, tips, …)
│     └─ pages/                 one module per page / tool
└─ llms/AGENTS.md               this file
```

Ignored by git: `target/`, `.env*`, and the Python environment folders in case `MODELOID_HOME` points into the repo (`modeloid/.venv/`, `python/`, `.uv-cache/`, `bin/`, `weights/`, `.ultralytics/`). Normally the environment lives outside the repo (§7). The maintainer also keeps private planning notes in an ignored `.schematic/` folder; you won't have it, and everything you need from it is in this file.

---

# 4. Build, run, test

Always run commands **from the repo root**. `dx` 0.7 resolves `default-members` relative to the current folder and crashes when run inside `modeloid-gui/`.

```sh
# Install the Dioxus CLI (once)
cargo install dioxus-cli --version 0.7.10   # or newer 0.7.x

# Run the app in development (server + browser app, hot reload)
dx serve --platform web -p modeloid-gui

# Type-check BOTH sides of the GUI: the browser build and the server build
cargo check -p modeloid-gui
cargo check -p modeloid-gui --features server

# Tests
cargo test -p modeloid-types
cargo test -p modeloid
cargo test -p modeloid-gui --features server

# Lints
cargo clippy -p modeloid
cargo clippy -p modeloid-gui --features server

# Release bundle (what users download; uses the size profiles in Cargo.toml)
dx bundle --release --platform web -p modeloid-gui
```

Notes:

- `default-members = ["modeloid-gui"]`: a bare `cargo run` / `cargo check` targets the GUI with only the `web` feature. Server code is behind `--features server`; **a change that compiles for one side may not compile for the other**. Check both.
- The Python environment is created by the app itself on first launch (Settings → Training Device). You don't need Python installed. Tests that need it are `#[ignore]`d (`core/transform.rs`), as is the one touching the OS credential store (`models/settings.rs`).
- Core tests make their own folders under `std::env::temp_dir()` named `modeloid-<area>-<name>-<pid>` and remove them. Follow that pattern; never write test data into the repo.
- API keys for development: save them in Settings → API Keys (OS credential store), or set them as environment variables before launching (`ROBOFLOW_API_KEY`, `HF_TOKEN`, `KAGGLE_API_TOKEN` / `KAGGLE_USERNAME` + `KAGGLE_KEY`; names in `models/settings.rs`). The app doesn't read `.env` files.
- `MODELOID_HOME` overrides where the Python environment root lives (see §7).

---

# 5. Architecture

## 5.1 Three crates, one direction

```
modeloid-types  ◄──  modeloid (core)  ◄──  modeloid-gui (server side, feature "server")
       ▲                                         │
       └───────────────  modeloid-gui (browser side, WASM)
```

- **`modeloid-types`**: plain data types and validation rules shared by the core and both sides of the GUI. **Serde only, no I/O, no heavy deps**: it must compile to WASM. Plans and previews of every tool (`ConvertPlan`, `BalancePlan`, `MergePlan`, `HealthReport`, `ExploreStats`, `TrainPreview`, `RunResults`, …), `formats::FORMATS`, `colors`, `validate_dataset_name`, `validate_api_key`. The core re-exports these.
- **`modeloid` (core)**: **all logic lives here**: reading/writing labels, scanning, downloads, tools, training orchestration, Python env, settings. It never draws UI; it reports progress through the `Reporter` / `Progress` traits (`core/progress.rs`) and checks `is_cancelled()` in long loops, returning `Cancelled`.
- **`modeloid-gui`**: Dioxus fullstack app. Server functions in `src/api/` are a **thin layer**: validate input, call the core, map results for the UI. **Don't put feature logic in `api/` or in pages.** If the browser needs a rule (validation, a format list, a colour), put it in `modeloid-types`.

Errors: the core uses `modeloid::Error = Box<dyn std::error::Error + Send + Sync>` (must be `Send + Sync` so futures move between threads). The GUI's server code maps errors to `String` messages written for the user, then to `ServerFnError`; the UI shows them with `api::error_message(&e)` (strips the "error running server function" wrapper).

## 5.2 Core modules (`modeloid/src`)

| Module | Responsibility |
| --- | --- |
| `models/workspace.rs` | `Workspace`: create / load / save, layout, `find_dataset`, `list_datasets`, `delete_dataset`, `renumber_datasets`, tool output (`staging_dir` → write → `save_output` / `save_described`), `Output::{New, Overwrite, Link}`, `fingerprint` (used by the GUI watcher) |
| `models/dataset.rs` | `Dataset` (`dataset.toml`): stats, splits, classes, `class_colors`, `lineage { operation, parents, params }`, `linked_to` |
| `models/parameters.rs` | training parameter sets in `training/parameters/<name>.toml` |
| `models/settings.rs` | user settings in `<OS config dir>/modeloid/settings.toml` (shared by every workspace); API keys go to the OS credential store (service `modeloid`) |
| `core/labels.rs` | one in-memory `LabelSet` for every format; readers/writers for YOLO, COCO, COCO segmentation, Pascal VOC, VIA; pixel coordinates; polygons kept |
| `core/scan.rs` | detects dataset type and format, collects stats |
| `core/download.rs` | Roboflow, Kaggle, Hugging Face, direct zip; ranged parallel downloads; every network wait cancellable |
| `core/io.rs` | zip extraction, moving folders, `rename` / `retry_busy` (Windows file locks) |
| `core/edit.rs` | create dataset, edit details, add images, uploads |
| `core/annotate.rs` | Labelling page edits in place (per-image label save, add/rename/remove classes) |
| `core/stats.rs`, `core/thumbs.rs` | Explore statistics (cached), gallery pages, image previews |
| `core/health.rs` | Health Check findings (corrupt images, bad boxes, duplicates via dHash, split leaks, …) and fixes |
| `core/classes.rs` | Classes & Merge: suggest, preview, apply; saved class lists |
| `core/balance.rs` | Split & Balance (own seeded SplitMix64 RNG, stratified split with exact totals) |
| `core/convert.rs` | format / task conversion, classification crops |
| `core/transform.rs` | Augmentation via `main.py augment` |
| `core/train.rs`, `core/tune.rs`, `core/performance.rs` | training runs, Optuna tuning, reading a run's results |
| `core/pyenv.rs` | the Python environment (locate root, uv setup, NVIDIA detection, child-process runner) |
| `core/limits.rs` | threads, download connections, jobs at once, machine cores/memory |
| `core/history.rs` | workspace history log (`.cache/history/history.jsonl`) |
| `core/progress.rs` | `Reporter`, `Progress`, `Unit`, `Cancelled`, `Silent` |

## 5.3 GUI (`modeloid-gui/src`)

**Routes** (`routes.rs`): `/`, `/workspace/create`, `/workspace/edit`, `/settings`, `/label/:id/:index`, and **every tool page shares `/:section/:tool`**, looked up in `nav::NAV` and dispatched in `pages/tool.rs`. To add a tool: add a `NavItem` in `nav.rs`, a page module in `pages/`, and a match arm in `pages/tool.rs`.

**Server functions** (`api/*.rs`), always in this shape:

```rust
/// What it does, in one sentence.
#[post("/api/convert/run")]
pub async fn start_convert(plan: ConvertPlan, output: OutputTarget) -> Result<Job, ServerFnError> {
    crate::api::blocking(move || backend::run(plan, output)).await
}

#[cfg(feature = "server")]
mod backend {
    // Server-only code: uses the `modeloid` core, returns Result<_, String>.
    use crate::api::workspace::open;
    pub fn run(plan: ConvertPlan, output: OutputTarget) -> Result<Job, String> {
        let ws = open()?;
        // validate, preview, then jobs::start(...)
    }
}
```

- The public `#[post]` fn compiles on both sides; everything touching the core, filesystem or OS goes in a `#[cfg(feature = "server")] mod backend`.
- Disk / TOML / keyring work runs through `api::blocking(...)` (`spawn_blocking`), so async threads stay free for requests and long-polls.
- Open the current workspace with `api::workspace::open()`.

**Jobs** (`api/jobs.rs`): long work (download, convert, train, …) is a job: `jobs::start(kind, item_labels, touches, work)`. A job is a list of items run in order on a blocking thread. Admission rules: one job per tool (`JobKind`) at a time; two jobs never touch the same dataset (`Touches::Datasets`); at most "jobs at once" (Settings → System). Items end Done / Failed / Warning / Skipped / Cancelled and can be re-run. Every job is written to History when it ends (`JobKind::history_path`). Job state is in server memory (lost on restart).

**Background loads** (`api/activity.rs`): slow reads (previews, stats) use `activity::track(slot, label, work)`. A newer load in the same slot cancels the older; loads under ~250 ms never show in the status bar.

**Progress to the browser**: **one app-wide long-poll** (`activity_status`, in `state::provide_activity_state`) carries every job and load, plus the workspace watcher (`api/watch.rs`, polls `Workspace::fingerprint`). Browsers allow only 6 open HTTP/1 requests per server; **never add another long-poll per component**. Pages read jobs with `use_job(kind)`.

**State** (`state.rs`, context): `WorkspaceState` (workspace info, `selected_datasets` as a `Vec` in selection order, `datasets_version`, selected parameters / models and their versions), `AppearanceState`, `ActivityState`. Single-item pages use the **last** selected item (`use_primary_dataset`, `use_primary_parameters`). Bump a `*_version` signal to make tables and pages reload.

**Files**: images are served by plain GET routes in `api/files.rs` (`/files/thumb/{id}/{index}`, `/files/image/{id}/{index}`, `/files/run/{name}/{*path}`), restricted to the workspace's own dataset / run folders.

**Launching and quitting**: outside `dx serve` (`DIOXUS_CLI_ENABLED` unset), `launcher.rs` runs before the server: one copy per user (`<config dir>/modeloid/instance` holds port + token, `GET /api/instance` answers the token, so a second launch only opens the browser on the running copy), port = `PORT` env, else the last run's, else 8080, else a free one; the browser opens once the port listens. The app runs in its console window; closing it stops the app. Python jobs stop with it through a lifeline: `pyenv::stream` keeps the child's stdin open and `main.py` exits when it closes (`MODELOID_LIFELINE=1`), so keep running main.py commands through `service_command` + `stream`.

**Native dialogs**: the browser can't give folder paths, so file/folder pickers open on the server side with `rfd` (`api/dialogs.rs`), brought to the front, run through `main_thread::run` (macOS needs the main thread; the server runs on a side thread there).

**Uploads**: a server fn taking `dioxus::fullstack::FileStream` (see `api/edit.rs`).

---

# 6. Data on disk

## 6.1 Workspace

```
<workspace>/
├─ workspace.toml        name, path, seed, description, default_format
├─ datasets/             one folder per dataset, each with a dataset.toml
├─ classes/              saved class lists (<name>.toml)
├─ training/             (older workspaces' models/ is renamed to this on load)
│  ├─ parameters/        parameter sets, <name>.toml (framework-tagged)
│  ├─ trains/<run>/      train.toml, parameters.toml, data.yaml, config.json, console.log, runs/ (Ultralytics output)
│  └─ tuning/<run>/      tune.toml, parameters.toml, data.yaml, config.json, console.log, trials/
├─ evals/                (not used yet)
└─ .cache/               tmp/ (downloads, staging, uploads), stats/, health/, thumbs/, images/, history/
```

Rules:

- **Everything happens inside the workspace.** Downloads, imports and tool results always land in `datasets/`; temporary files in `.cache/tmp/`. No custom target folders.
- **Tool output is staged**: write into `staging_dir(op)` under `.cache/tmp/`, then move into `datasets/` by rename (same drive, atomic-ish). Overwrites swap through a backup.
- Every tool asks for an **output**: `Output::New { name }` (default) or `Output::Overwrite { id }` (confirmed by the user; not allowed for multi-input merges). Classes & Merge also offers `Output::Link` (hard-linked images, see §10).
- **Lineage**: every operation records `operation`, `parents` and its settings in the result's `dataset.toml`.
- `dataset.toml` is generated by the scanner; there is no manual "fix metadata" step. Stats are refreshed by a Rescan.
- Dataset ids are `1..n`; `renumber_datasets` keeps them dense (only while no tool runs); caches and links follow.
- Clear cache removes `.cache/` contents, including history.

## 6.2 Settings and secrets

- `<OS config dir>/modeloid/settings.toml`: appearance, system limits, last folder per dialog, train device. Shared by all workspaces.
- **API keys never go in settings.toml** (when a credential store exists). They live in the OS store (service `modeloid`; entries `roboflow-api-key`, `kaggle` (JSON), `huggingface-token`). Keys an older version left in the file move to the store on load. The browser only ever receives a "Saved" status, never the key.

## 6.3 Label formats

`modeloid_types::formats::FORMATS` is the single list of annotation formats (≈40 known; readable/writable ones flagged). Read and written today: YOLO (v5 … YOLO26 layouts), COCO, COCO segmentation, Pascal VOC, VIA. Others are download-only. Classification (class-folder) datasets are detected but not read by tools yet. Add new formats to `FORMATS` only, plus a reader/writer in `core/labels.rs`.

---

# 7. Python service

Rust never imports Python; it runs `python main.py <command> <config.json>` as a child process inside the managed venv and reads its stdout.

Two folders, kept apart (`PyEnv::locate`), because installed apps can't write to their own folder (Program Files, a signed macOS .app, an AppImage):

- **Service** (only read, part of the app): `pyproject.toml`, `uv.lock` (committed), `main.py`. `pyservice/` next to the executable when installed; `modeloid/src/pyservice` in development (debug builds only, so release binaries don't carry the builder's path).
- **Root** (everything setup writes), in the user's local data folder: `%LOCALAPPDATA%\Modeloid` (Windows), `~/Library/Application Support/Modeloid` (macOS), `~/.local/share/modeloid` (Linux), also in development. Deleting it removes the environment; the next launch sets it up again.
- `MODELOID_HOME=<dir>` puts both in one folder (`<dir>/src/pyservice` + the root's folders), e.g. for a scratch environment.

```
<root>/
├─ .venv/                 the environment; .venv/modeloid-env.json records what was installed
├─ python/                uv-managed CPython 3.12
├─ .uv-cache/             uv's cache (cleaned after installs)
├─ bin/uv(.exe)           bundled, else `uv` on PATH, else downloaded (pinned version, SHA-256 checked)
├─ weights/               Python's working dir: pretrained weights download here
└─ .ultralytics/          YOLO_CONFIG_DIR
```

- Setup runs automatically on launch when needed (`pending_install()`): Python + Ultralytics + Optuna + Albumentations + huggingface_hub + the **CPU** PyTorch build. Half-finished setups are repaired, outdated ones (changed `uv.lock`) updated.
- The only user change is CPU ↔ GPU: `uv sync --locked --extra cpu|cu130`, which swaps only `torch`/`torchvision`. The base libraries can't be removed and the env is never deleted (separate install/remove steps broke environments before).
- Always run uv with `UV_PYTHON_INSTALL_DIR=<root>/python`, `UV_CACHE_DIR=<root>/.uv-cache`, `UV_PYTHON_PREFERENCE=only-managed` (as `PyEnv::uv` does), or uv uses the global Python folder.
- The pinned uv version and its SHA-256 (`fetch_uv` / `uv_sha256` in `core/pyenv.rs`) are **updated together**.
- When PyTorch moves to a new CUDA line: change the index + extra in `pyproject.toml` and `TorchVariant::extra`, then `uv lock`.

**Protocol** (`main.py` docstring is the source of truth): commands `check` (prints one JSON object), `train`, `tune`, `augment`, `hf-download`, each taking a JSON config file. Library output passes through as console text; Modeloid's events are lines of the form

```
@modeloid {"event": "epoch", ...}
```

(start, epoch, trial, image, progress, done, error). Errors exit non-zero. If you add a command or event, update the docstring and the Rust reader in the matching `core/*.rs` together.

Python rules:

- Always `open(..., encoding="utf-8")` (Windows defaults to cp1252).
- OpenCV: read with `cv2.imdecode(np.fromfile(path, np.uint8), flags)`, write with `cv2.imencode(ext, img)[1].tofile(path)`; never `cv2.imread` / `imwrite` (they fail on non-ASCII paths on Windows).
- Only `opencv-python-headless` is installed (`override-dependencies` in `pyproject.toml`); don't add a dependency that pulls GUI OpenCV back.
- Train/tune clear Ultralytics' git info (`forget_git()`), so checkpoints don't carry the user's repo path, branch or remote.

---

# 8. Security model

The server runs with the user's rights on `localhost`, so any website the user visits could try to call it. Keep these guarantees:

- **`api/guard.rs`** wraps every route: refuses a non-loopback `Host` (DNS rebinding), a non-local `Origin` on state-changing methods (CSRF), and `Sec-Fetch-Site: cross-site|same-site` on `/api/*` and `/files/*`. Don't add routes outside the router it wraps. Under `dx serve` the dev proxy rewrites `Host`, so test the Host check in a release build.
- **Paths from the browser are untrusted.** File routes and anything taking a name/path must stay inside the workspace's own folders: no `..`, canonicalize and check the prefix (see `Workspace::run_image`). Run names, dataset names and "replacing" names are validated (`validate_dataset_name` etc.).
- **Secrets**: keys only in the OS credential store; never send a key to the browser, never log it, never put it in history or error messages (beware URLs in `reqwest` errors that contain query-string keys).
- **Downloads of executables** (uv) are checksum-pinned.
- Don't add telemetry, analytics, remote fonts/CDNs or any network call the user didn't ask for.
- Never write personal data (usernames, absolute home paths, emails) into files that could be shared (datasets, checkpoints, logs meant for export).

---

# 9. UI design rules

These hold for every page, old and new. Check your UI change against them.

## Foundations

- **Tokens only.** Spacing, type sizes, radii, motion and layout sizes are CSS tokens at the top of `assets/main.css`; colours and fonts are tokens in `assets/themes.css`. Rules use `var(--…)`; no raw colours, no raw sizes when a token fits.
- **Spacing scale** (2/4 px): `--space-0-5` 2, `--space-1` 4, `--space-2` 8, `--space-3` 12, `--space-4` 16, `--space-5` 20, `--space-6` 24, `--space-8` 32.
- **Type**: use `--text-xs/sm/base/xl/2xl`, which follow the text size chosen in Settings (`--font-scale` via `data-scale`). Never a fixed `font-size` that ignores the scale.
- **Fonts**: JetBrains Mono first, plus Inter, IBM Plex Sans, Atkinson Hyperlegible Next. All bundled (OFL). Files used from CSS must be declared with `asset!()` in Rust and the CSS injected (see `components/fonts.rs`), because Dioxus hashes asset names and bundles only `asset!()` files.
- **Themes**: 16 (8 dark, 8 light), "Lime" dark is the default. A new theme fills **every** token listed at the top of `themes.css` and is added to `THEMES` in `src/api/appearance.rs`. Contrast: `--color-fg` / `--color-fg-muted` ≥ 4.5:1 on bg and surface; `--color-accent-on` ≥ 4.5:1 on accent; `--viz-*` ≥ 3:1 on surface.
- **Icons**: inline SVG from `components/icon.rs` (`Icon { name: IconName::… }`); add new ones there (Lucide-style line icons). In rsx use string attribute names for SVG ARIA: `"aria-hidden": "true"`.
- **Motion**: short (`--duration`, `--ease-out`), respect `prefers-reduced-motion`. No timers for UI animation (toasts leave via `animationend`).

## Layout

- **Pages**: `.page` (960 px) or `.page.page-wide` (1200 px), with `.page-eyebrow` (sidebar group), `h1`, and a one-sentence `.page-lead`.
- **Panels** (`.section-card`): bordered surface cards, 16 px padding and inner gap; they stretch with the content column, never a fixed width.
- **Spacing between things**: cards in a `.section-stack` have a 20 px gap; anything after a form or button row keeps that gap. The "gap above a panel" rule must not hit grid children (`.chart-grid`); grid cells in a row are equal height, no holes, content top-aligned.
- **Titles inside panels**: every card's title sits inside its border (`.section-card-title`, also for `fieldset` legends) and starts with a `TitleIcon`. Same meaning, same icon: Result → Chart, Output → Save, Inputs / Datasets → Database, Options / parameters → Sliders, classes → Tag, images → Image.
- **Tabs** inside a page: `.segmented.segmented-tabs`.
- **Side panel**: resizable (`ResizeHandle`, min 360 px `PANEL_MIN_WIDTH`), never makes the window scroll; switches tables (Datasets / Parameters / Models) from its title. **Tools take input from the side panel selection; no dataset pickers on tool pages.** A page needing a pick from another table switches the panel (`use_panel_table`) with a "Choose in the … table" button; a page working on one kind of row opens the panel on that table (`PanelTable::for_tool`).
- **Nothing selected**: show exactly `SelectDatasetHint` ("Select a dataset in the datasets panel" + arrow) / `SelectModelHint`. No workspace: `NoWorkspace` with Create and Load.
- **Mobile**: below 768 px the sidebar is a drawer; no horizontal scroll.

## Components and patterns

- **Tables** (all of them): build from `components/table.rs` (`Column`, `Sort`, `SortHeader`, `SelectionBar`, `RowCheck`, `CheckHeader`, …). Checkbox column; row click selects only that row (again: clears); `.row-pick` name button for keyboard; Select all / None / red **Remove** (confirm "Delete N …? … can't be undone" in a small popup under the button) / "N selected"; `#` id column; sortable headers with `aria-sort` (text A–Z first, counts largest first, ids lowest first); count + reload button in the header. Selection is a `Vec` in selection order. Deleting is only Remove in the table, no separate delete pages.
- **Fixed lists** (e.g. Tune's search space): checkbox = included, Select all / None / count, no Remove, reload icon resets defaults. **Result tables**: sortable with `#` and a count, no checkboxes.
- **Table cells stay table cells**: never `display: flex` on `td`/`th`; put a flex `span` inside.
- **Rows with many settings**: settings go on a line under their row in the same `tbody` band; the table never scrolls sideways.
- **Popups escape their containers**: tips and popups are placed against the window (`InfoTip` does this), never clipped by scroll boxes.
- **Explanations are hover icons**: use `InfoTip` next to a label, not lines of small text. Small text only for live values, errors, and safety notes that must be seen.
- **Warnings**: a `WarningTip` icon in the card header listing them, not a wall of notices.
- **Numbers with a sensible range** use `Slider` (value shown next to it, optional text like "Automatic" for 0). Free-range numbers use `NumberInput` / `DecimalInput`.
- **Form grids**: one column per field, number boxes and dropdowns the same width; checkbox rows on the same grid; a modifier option (e.g. batch "Auto") sits right of its box.
- **Choices**: radio cards (`.choice-list` / `.choice`), whole card clickable, action buttons below the card.
- **Picking several names**: `.chip-list` of `.chip` toggles (`aria-pressed`); none picked = all, and then every chip shows as picked.
- **Only offer what applies**: options that can't apply to the input are hidden, not disabled. Use disabled + a reason only when the user can change something to make it apply. Exception: write-side format pickers show every format, unsupported ones disabled "(not supported yet)".
- **Annotation formats**: every format dropdown is `FormatSelect`, fed by `modeloid_types::formats`.
- **Class colours**: a class looks the same everywhere (boxes, outlines, tags, swatches) using `Dataset.class_colors` / `modeloid_types::colors`; tags on class colours use dark text (#111). Colours are changed only in Classes & Merge with `ColorPicker`; never a bare `input type=color`.
- **Tool output**: `OutputSelect` ("Save as new dataset" default / "Overwrite source" with an "I understand … can't be undone" checkbox). The new name is a full name prefilled `<source>-<suffix>` (e.g. `-fixed`), one per input; never ask for a suffix alone.
- **Suggested names** are short (`<dataset>-<model>`), numbered when taken; never warn about a name the app itself just suggested.
- **Picking files**: one form takes a folder or a zip ("Choose folder" / "Choose zip"), told apart by the `.zip` path.
- **Pictures**: any image set is shown like Explore's gallery (`.gallery-grid` of `.gallery-tile` with captions; click opens the detail viewer, ←/→ step, Esc closes).
- **Logs / consoles** are collapsible and closed by default; the progress panel carries the status.
- **Status pills**: rounded border + matching soft background. Done/installed = green (accent), Failed = red, Warning = yellow, Skipped = grey.
- **Form actions** (`.form-actions`): primary button first, then ghost buttons; notes beside them vertically centred.
- **Job results on pages**: a page shows the job it is running or just ran; after leaving and coming back, finished jobs are gone from the page and live in History.
- **Feedback**: toasts (`use_toasts`) for finished actions; the status bar shows running jobs and loads with a spinner and Stop.
- **Optional helper sections** (e.g. "Prepare from dataset") are open by default.

## Charts

- Only `--viz-*` tokens. Components in `components/charts.rs`: `BarList`, `HistogramChart`, `Heatmap`, `LineChart`, `StatTile`.
- Line charts: small multiples (one quantity per chart); a train/val pair shares a hue (solid / dashed) with a legend; labelled y min/max and first/last x; hover column per x; dotted marker for a highlighted x and dotted level line for a reference value. Numbers via `format_value`.
- Every chart has an accessible name; bars have a `data-tip` with the exact count; a clickable bar is a `button`.

## Text

- Plain, short English that says what happens ("Images are copied into the dataset's images folder"). Sentence case for labels and titles. Buttons name the action ("Add dataset", "Convert", "Remove"); while working: "Adding…", "Working…".
- Destructive confirmations name the count and say it can't be undone.
- Numbers through `format_count`, plurals through `plural` (`util.rs`).

## Accessibility

- Every control has a visible label or `aria-label`; tips, warnings and sliders carry their text for screen readers (`aria-label`, `aria-valuetext`).
- Visible `:focus-visible`, full keyboard use, `role="status"` / `role="alert"` for live messages, `aria-sort` / `aria-pressed` / `aria-checked` where they apply.
- Don't rely on colour alone (pills also say it in words).

---

# 10. Coding conventions

- **Match the surrounding code.** Comment density, naming and idiom of the file you're in win over your defaults.
- **Comments and docs in plain, short English** that explain *why* and *what happens*, written for a reader who doesn't know the code (see `api/jobs.rs`, `api/guard.rs`, the `Cargo.toml` comments). Every module starts with a `//!` line saying what it is. Every non-obvious dependency in a `Cargo.toml` gets a comment saying why it's there.
- **Logic in the core**, types/rules in `modeloid-types`, `api/` stays thin, pages render.
- Error messages are for the user: say what went wrong and what to do ("images can't be linked on this drive … use Save as new dataset"). No debug dumps, no secrets, no stack traces.
- Long loops check cancellation (`reporter.is_cancelled()` / `check_cancel`); network waits are cancellable.
- Heavy per-image work goes through `limits::run_parallel` so sections share the configured thread count.
- New random steps take a seed (the tool's own, else the workspace seed) and must be reproducible.
- Dioxus:
  - Follow `modeloid-gui/AGENTS.md` (0.7 API).
  - `clippy.toml` forbids holding signal read/write guards across `.await`; read into a local first.
  - rsx text can't nest `format!` calls: build the string in a `let` first, then `"{note}"`.
  - A resource is `None` while reloading, so children rendered from it unmount; keep the last loaded value in a signal (via `use_effect`) and render from that when a form must keep its state.
  - Client-side delay without a timer crate: `document::eval("await new Promise(r => setTimeout(r, 1500));").await`.
  - Code using browser APIs runs after hydration (`use_effect`); first-paint data comes from `use_server_future`.
- Windows: renames/deletes may fail with "Access is denied" while a scanner or viewer holds a file; use `io::rename` / `retry_busy`.
- Keep the release small: don't add a second HTTP/TLS stack, big optional features, or `panic = "abort"` (a panicking request would close the app). Check duplicates with `cargo tree -p modeloid-gui --no-default-features --features server -d`. Size work is about the shipped bundle, not dev `target/` size.
- Formatting: `cargo fmt`; lines in this codebase are often long (up to ~140 chars), so don't reflow existing code in unrelated edits.

---

# 11. Recipes

**Add a dataset tool**

1. Types for its plan/preview in `modeloid-types/src/<tool>.rs` (serde only), re-exported where needed.
2. Logic in `modeloid/src/core/<tool>.rs`: `preview(...)` (fast, often from cached stats) and `apply_<tool>(plan, output, reporter)` writing through staging + `save_output` / `save_described` with lineage. Tests with temp folders.
3. `modeloid-gui/src/api/<tool>.rs`: `<tool>_preview` (via `activity::track`) and `start_<tool>` (via `jobs::start`, a new `JobKind` with `label` and `history_path`).
4. Page in `modeloid-gui/src/pages/<tool>.rs`: input from the side panel selection, `OutputSelect`, `JobProgress`, design rules from §9.
5. Register in `nav.rs` and `pages/tool.rs`.

**Add a label format**: entry in `modeloid_types::formats::FORMATS`; reader/writer in `core/labels.rs`; detection in `core/scan.rs`; tests with small fixtures written in the test.

**Add a theme**: fill every token in `themes.css` for `[data-theme="<id>"]`, add to `THEMES` in `api/appearance.rs`, check contrast numbers in §9.

**Add a Python command**: function in `main.py` + docstring entry, events as `@modeloid {json}` lines, a Rust caller in the matching core module using `pyenv` streaming; if a new library is needed, add it to `pyproject.toml` and run `uv lock` (existing environments update on launch).

---

# 12. Known gaps and limitations

- Formats: only YOLO / COCO / VOC / VIA families are read and written; the rest are download-only. Classification (class-folder) datasets are listed but not read by tools. Hugging Face parquet-only datasets scan as "unknown".
- Job state is in memory: lost on server restart; refused jobs aren't queued. A run whose server stopped mid-training stays "running".
- Local only: `IP=0.0.0.0` serving to other machines is refused by the guard, and native dialogs open on the server machine.
- Kaggle downloads aren't tested live. Cancelling a Roboflow fork while Roboflow is still copying can't delete the copy.
- Augmentation: no SAHI-style tiling, no sample preview yet.
- Release packaging isn't set up: `modeloid/src/pyservice/` must ship as `pyservice/` next to the executable; `bin/uv` is not shipped (downloaded on demand into the root).
- Model evaluations (`evals/`) not started.

---

# 13. Solved traps (don't re-solve them)

- **`dx` crashes inside `modeloid-gui/`**: run from the repo root with `-p modeloid-gui`.
- **Core futures weren't `Send`**: keep `modeloid::Error` `Send + Sync`; jobs run via `spawn_blocking`.
- **Browser can't use the core** (reqwest/tokio/zip/image don't build for WASM): shared things go in `modeloid-types`.
- **Pages stalled waiting on requests**: browsers cap open requests at 6 per server; there's one activity long-poll + the watcher. Don't add more.
- **Serving files from Dioxus 0.7**: `dioxus::serve` with an axum router merged into `dioxus::server::router(App)`; handlers in `api/files.rs`.
- **Native dialogs behind the browser (Windows)**: a 1×1 topmost owner window attached to the foreground thread (`api/dialogs.rs::front`).
- **macOS dialogs panic "RFD in NonWindowed environment" / busy cursor**: dialogs run on the main thread from a queue and the idle loop pumps AppKit events (`main_thread.rs`).
- **Loaded workspace used the wrong folder**: `load_from_dir` sets `path` to the folder it loaded from (workspaces may store `path = "."`).
- **Roboflow "has no versions"**: on request, fork → generate a version → download → delete the copy. **"X is an invalid format for project type Y"**: parse the allowed formats from the 400 answer and fall back.
- **Two OpenCV packages**: only the headless one is kept via `override-dependencies`.
- **CUDA 12.8 index stops at an old torch**: use `cu130` (driver ≥ 580, compute ≥ 7.5).
- **Tuning trials kept ~10 MB of weights each**: `main.py tune` deletes a trial's `weights/` after reading its score.
- **Dataset links**: hard links (same drive only; otherwise stop with a clear message, never silently copy); a link's `size_bytes` leaves out image bytes.
- **Releases were too big**: one reqwest/rustls(ring) stack, size profiles, `wasm_opt` level z, `uv cache clean` after installs. `dx bundle` doesn't empty `target/dx/modeloid-gui/release/web/public`; delete it before measuring.
- **Shell heredocs break on quotes** in scripted edits: write the script to a temp file and run it.
- **Testing the GUI without browser tools**: headless Edge/Chrome with `--remote-debugging-port` driven over the DevTools protocol from a small Node script; set up data through server functions first.

---

# 14. Before you open a pull request

1. `cargo check -p modeloid-gui` **and** `cargo check -p modeloid-gui --features server` pass.
2. Tests pass: `cargo test -p modeloid-types`, `cargo test -p modeloid`, `cargo test -p modeloid-gui --features server`. New core logic has tests.
3. `cargo clippy` is clean for the crates you touched.
4. UI changes: looked at in `dx serve --platform web -p modeloid-gui`, in one dark and one light theme and at a large text size; spacing, alignment and equal heights match §9; keyboard and screen-reader labels work.
5. No new network calls, telemetry, CDN assets, secrets in logs, or paths escaping the workspace (§8).
6. If you changed the Python protocol, `main.py`'s docstring and the Rust reader agree; if you changed `pyproject.toml`, `uv.lock` is regenerated and committed.
7. If you added a project-wide rule, convention or solved a non-obvious problem, add it to this file in the matching section.
8. Commits: short imperative subject saying what changed; keep unrelated reformatting out of the diff.
