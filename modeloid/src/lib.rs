/// Error type of the core. `Send + Sync`, so core futures and results can move
/// between threads (e.g. run inside a web server's tasks).
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub mod models;
pub mod core;

pub use models::dataset::{Dataset, Lineage, ParentRef};
pub use models::workspace::{ConvertSummary, ImportMode, Output, validate_dataset_name};
pub use models::settings::{AppearancePrefs, KaggleCredentials, Service, Settings, validate_api_key, HF_TOKEN_ENV, KAGGLE_KEY_ENV, KAGGLE_USERNAME_ENV, ROBOFLOW_KEY_ENV};
pub use models::workspace::Workspace;
pub use core::download::{AlreadyExists, DownloadOptions, Downloader, Inspected, NoVersions, Source, SourceInfo};
pub use core::scan::{DatasetStats, scan_dataset};
pub use core::labels::{BBox, ImageLabels, LabelFormat, LabelSet, Object, Split};
pub use core::stats::{ExploreStats, GalleryFilter, GalleryImage, GalleryObject, GalleryPage, Histogram};
pub use core::limits::SystemPrefs;
pub use core::progress::{Cancelled, Progress, Reporter, Silent, Unit};
