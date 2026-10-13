<!-- Improved compatibility of back to top link: See: https://github.com/othneildrew/Best-README-Template/pull/73 -->

<a id="readme-top"></a>

[![Contributors][contributors-shield]][contributors-url]
[![Forks][forks-shield]][forks-url]
[![Stargazers][stars-shield]][stars-url]
[![Issues][issues-shield]][issues-url]
[![License][license-shield]][license-url]

<!-- PROJECT LOGO -->

<br />
<div align="center">
  <a href="https://github.com/denizhoroz/modeloid">
    <img width="1600" height="400" alt="banner" src="https://github.com/user-attachments/assets/a4b4dd38-7c9c-4669-bc16-c341423a9c20" />
  </a>

<h3 align="center">Modeloid - A Local Computer Vision Workbench</h3>

  <p align="center">
    Modeloid is a local computer vision workbench that helps you from preparing datasets to training models and reviewing them.
    <br />
    <br />
    <a href="https://github.com/denizhoroz/modeloid/issues/new?labels=bug&template=bug-report---.md">Report Bug</a>
    &middot;
    <a href="https://github.com/denizhoroz/modeloid/issues/new?labels=enhancement&template=feature-request---.md">Request Feature</a>
  </p>
</div>

<!-- TABLE OF CONTENTS -->

<details>
  <summary>Table of Contents</summary>
  <ol>
    <li>
      <a href="#about-the-project">About The Project</a>
      <ul>
        <li><a href="#built-with">Built With</a></li>
      </ul>
    </li>
    <li>
      <a href="#getting-started">Getting Started</a>
      <ul>
        <li><a href="#prerequisites">Prerequisites</a></li>
        <li><a href="#installation">Installation</a></li>
      </ul>
    </li>
    <li><a href="#usage">Usage</a></li>
    <li><a href="#future-features">Future Features</a></li>
    <li><a href="#contributing">Contributing</a></li>
    <li><a href="#license">License</a></li>
    <li><a href="#contact">Contact</a></li>
    <li><a href="#acknowledgments">Acknowledgments</a></li>
  </ol>
</details>

<!-- ABOUT THE PROJECT -->

## About The Project

<img width="1920" height="917" alt="site-content0" src="https://github.com/user-attachments/assets/c1b74edb-8aae-4c9f-911c-c21ab269e34e" />
<br><br/>

Modeloid provides many features for your computer vision projects:

* **Get datasets** from Roboflow, Kaggle, Hugging Face or a direct link, import a folder or zip, or create one from scratch
* **Label images** with boxes right in the app
* **Explore** statistics and browse a gallery with label overlays
* **Health Check** finds corrupt images, missing labels, bad boxes, duplicates and split leaks, and fixes them
* **Classes & Merge** renames, merges and drops classes across datasets
* **Split & Balance** makes stratified splits, subsets and class caps
* **Convert** between YOLO, COCO, VOC and VIA, or turn detections into classification crops
* **Augment** datasets with 40 Albumentations transforms
* **Train** Ultralytics YOLO models on CPU or NVIDIA GPU, with live progress and resume
* **Tune** hyperparameters with Optuna and review every run's metrics and plots in **Performance**

and many more. Everything runs locally on your machine, in your browser, with 16 themes to choose from.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### Built With

* [![Rust][Rust-badge]][Rust-url]
* [![Dioxus][Dioxus-badge]][Dioxus-url]
* [![Python][Python-badge]][Python-url]

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- GETTING STARTED -->

## Getting Started

Modeloid ships as a ready-to-run app for **Windows**. Download it from the [Releases](https://github.com/denizhoroz/modeloid/releases) page, start it, and it opens in your default browser.

**Linux and macOS have no prebuilt downloads.** Modeloid runs on both, but you need to build it yourself (see [Building from source](#building-from-source-linux-and-macos)).

### Prerequisites

* Windows 10/11 (x64), or Linux (x64) / macOS (Apple Silicon) if you build it yourself
* A modern browser (Chrome, Edge or Firefox)
* An internet connection the first time you train, tune or augment. Modeloid then sets up its own Python environment (about 1 GB for CPU, about 3.4 GB for GPU). You don't need to install Python yourself, and Modeloid doesn't use a Python you already have.
* Optional: an NVIDIA GPU (compute capability 7.5 or newer, driver 580 or newer) for faster training

### Installation

#### Windows

**Installer (recommended)**

1. Download `Modeloid-<version>-setup.exe` from [Releases](https://github.com/denizhoroz/modeloid/releases)
2. Run it and follow the steps
3. Start Modeloid from the Start menu

**Portable zip**

1. Download `Modeloid-<version>-windows-x64.zip` from [Releases](https://github.com/denizhoroz/modeloid/releases)
2. Extract it anywhere
3. Run `Modeloid.exe`

Modeloid runs in its console window and opens `http://localhost:8080` (or another free port) in your browser. Closing the window stops the app.

#### Building from source (Linux and macOS)

Linux and macOS builds aren't tested or supported the way the Windows release is. If something breaks, please [open an issue](https://github.com/denizhoroz/modeloid/issues) with the error.

**1. Install the build tools**

Linux (Debian/Ubuntu; use your distribution's equivalents elsewhere):

```sh
sudo apt install -y build-essential pkg-config libgtk-3-dev libdbus-1-dev libssl-dev librsvg2-bin
```

macOS:

```sh
xcode-select --install
brew install librsvg
```

**2. Install Rust and the Dioxus CLI**

Use these exact versions: newer Rust releases break the browser app build.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup toolchain install 1.98.1 --target wasm32-unknown-unknown --component llvm-tools
rustup default 1.98.1
cargo install dioxus-cli@0.7.10 --locked
```

Building the Dioxus CLI takes a while and uses a lot of memory. If it stalls or your system freezes, run it again with fewer parallel jobs: `cargo install dioxus-cli@0.7.10 --locked -j 2`.

**3. Build Modeloid**

```sh
git clone https://github.com/denizhoroz/modeloid
cd modeloid
bash installer/package.sh linux-x64     # or: macos-arm64
```

The result is in `dist/`:

* **Linux:** `dist/Modeloid-<version>-linux-x64/`. Move the folder anywhere and run `./modeloid` from it. `./install.sh` adds Modeloid to your application menu (`./install.sh --remove` takes it out). File dialogs need GTK 3, present on most desktop distributions.
* **macOS:** `dist/Modeloid.app`. Move it to Applications and open it. It runs in a Terminal window; closing that window stops the app. The build isn't signed, so if macOS blocks it, run `xattr -dr com.apple.quarantine /Applications/Modeloid.app` once.

The build needs about 10–20 GB of free space. Once you've copied the app out of `dist/`, you can delete the `modeloid` folder.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- USAGE EXAMPLES -->

## Usage

<img width="1920" height="917" alt="screenshot-2026-10-10-183329" src="https://github.com/user-attachments/assets/0669f6bc-595c-4a56-b2b1-9f7362bb4b21" />
<br><br/>

A typical project, from raw images to a trained model:

1. **Open a workspace.** On first launch, create a workspace folder (or open an existing one). Datasets, parameter sets and training runs are all stored there.
2. **Add your API keys (optional).** In **Settings → API Keys**, add your Roboflow, Kaggle or Hugging Face keys to download from those sites. Keys are saved in your system's credential store.
3. **Get a dataset.** In **Datasets**, download one, import a folder or zip, or create an empty one and label it in the app.
4. **Look at your data.** **Explore** shows class counts, box sizes and a gallery with label overlays.
5. **Clean it up.** Run **Health Check** to find and fix broken images and labels, then use **Classes & Merge**, **Split & Balance**, **Convert** and **Augmentation** to shape the dataset you need. Each tool saves its result as a new dataset, so the original stays untouched.
6. **Choose your training device.** In **Settings → Training Device**, pick CPU or GPU. Modeloid installs the Python environment it needs.
7. **Train.** In **Training → Parameters**, create a parameter set (model, epochs, augmentation…), then start a run in **Train Model** and watch it live.
8. **Review and improve.** **Performance** shows each run's metrics, curves and training plots. **Tune Hyperparameters** searches for better settings with Optuna.
9. **Make it yours (optional).** In **Settings → Appearance**, pick one of 16 dark and light themes, a font and a text size.

<img width="1920" height="917" alt="appearance" src="https://github.com/user-attachments/assets/b38315c7-fba6-4078-a9d8-f17812e5eacb" />


<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- FUTURE FEATURES -->

## Future Features

Features that will be added in the future:

- [ ] Autolabelling
- [ ] Collaborative Labelling
- [ ] Polygon Labelling
- [ ] Cloud Databases and Processes
- [ ] Better Training Interface
- [ ] Automation
- [ ] CLI Support

See the [open issues](https://github.com/denizhoroz/modeloid/issues) for more proposed features and known issues.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- CONTRIBUTING -->

## Contributing

Contributions are what make the open source community such an amazing place to learn, inspire, and create. Any contributions you make are **greatly appreciated**.

If you have a suggestion that would make this better, please fork the repo and create a pull request. You can also simply open an issue with the tag "enhancement".
Don't forget to give the project a star! Thanks again!

1. Fork the Project
2. Create your Feature Branch (`git checkout -b feature/AmazingFeature`)
3. Commit your Changes (`git commit -m 'Add some AmazingFeature'`)
4. Push to the Branch (`git push origin feature/AmazingFeature`)
5. Open a Pull Request

<p align="right">(<a href="#readme-top">back to top</a>)</p>

### Top contributors:

<a href="https://github.com/denizhoroz/modeloid/graphs/contributors">
  <img src="https://contrib.rocks/image?repo=denizhoroz/modeloid" alt="contrib.rocks image" />
</a>

<!-- LICENSE -->

## License

Distributed under the GNU Affero General Public License v3.0. See [`LICENSE`](LICENSE) for more information.

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- CONTACT -->

## Contact

Deniz Horoz - [denizhoroz.ofcl@gmail.com](mailto:denizhoroz.ofcl@gmail.com)

Project Link: [https://github.com/denizhoroz/modeloid](https://github.com/denizhoroz/modeloid)

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- ACKNOWLEDGMENTS -->

## Acknowledgments

* [Ultralytics](https://github.com/ultralytics/ultralytics) - YOLO training
* [Optuna](https://optuna.org/) - hyperparameter tuning
* [Albumentations](https://albumentations.ai/) - image augmentation
* [Dioxus](https://dioxuslabs.com/) - the app's interface
* [uv](https://github.com/astral-sh/uv) - Python environment setup
* [Lucide](https://lucide.dev/) - icons
* [JetBrains Mono](https://www.jetbrains.com/lp/mono/), [Inter](https://rsms.me/inter/), [IBM Plex Sans](https://www.ibm.com/plex/) and [Atkinson Hyperlegible Next](https://www.brailleinstitute.org/freefont/) - fonts
* [Catppuccin](https://catppuccin.com/), [Rosé Pine](https://rosepinetheme.com/), [Tokyo Night](https://github.com/folke/tokyonight.nvim), [Gruvbox](https://github.com/morhetz/gruvbox), [Dracula](https://draculatheme.com/), [Nord](https://www.nordtheme.com/), [Everforest](https://github.com/sainnhe/everforest) and [Solarized](https://ethanschoonover.com/solarized/) - theme colours
* [Best-README-Template](https://github.com/othneildrew/Best-README-Template) - this README's layout

<p align="right">(<a href="#readme-top">back to top</a>)</p>

<!-- MARKDOWN LINKS & IMAGES -->

<!-- https://www.markdownguide.org/basic-syntax/#reference-style-links -->

<!-- Shields.io badges. You can a comprehensive list with many more badges at: https://github.com/inttter/md-badges -->

[contributors-shield]: https://img.shields.io/github/contributors/denizhoroz/modeloid.svg?style=for-the-badge
[contributors-url]: https://github.com/denizhoroz/modeloid/graphs/contributors
[forks-shield]: https://img.shields.io/github/forks/denizhoroz/modeloid.svg?style=for-the-badge
[forks-url]: https://github.com/denizhoroz/modeloid/network/members
[stars-shield]: https://img.shields.io/github/stars/denizhoroz/modeloid.svg?style=for-the-badge
[stars-url]: https://github.com/denizhoroz/modeloid/stargazers
[issues-shield]: https://img.shields.io/github/issues/denizhoroz/modeloid.svg?style=for-the-badge
[issues-url]: https://github.com/denizhoroz/modeloid/issues
[license-shield]: https://img.shields.io/github/license/denizhoroz/modeloid.svg?style=for-the-badge
[license-url]: https://github.com/denizhoroz/modeloid/blob/main/LICENSE
[product-screenshot]: images/screenshot.png
[Rust-badge]: https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white
[Rust-url]: https://www.rust-lang.org/
[Dioxus-badge]: https://img.shields.io/badge/Dioxus-00A8D6?style=for-the-badge
[Dioxus-url]: https://dioxuslabs.com/
[Python-badge]: https://img.shields.io/badge/Python-3776AB?style=for-the-badge&logo=python&logoColor=white
[Python-url]: https://www.python.org/
