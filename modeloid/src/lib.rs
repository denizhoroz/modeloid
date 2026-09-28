/// Error type of the core. `Send + Sync`, so core futures and results can move
/// between threads (e.g. run inside a web server's tasks).
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

pub mod models;
pub mod core;

pub use models::dataset::{Dataset, Lineage, ParentRef};
pub use models::workspace::{Output, validate_dataset_name};
pub use models::settings::{Settings, saved_roboflow_api_key, validate_api_key, ROBOFLOW_KEY_ENV};
pub use models::workspace::Workspace;
pub use core::download::{DownloadOptions, Downloader, Source, SourceInfo};
pub use core::scan::{DatasetStats, scan_dataset};
pub use core::labels::{BBox, ImageLabels, LabelFormat, LabelSet, Object, Split};
pub use core::progress::{Progress, Reporter, Silent, Unit};
