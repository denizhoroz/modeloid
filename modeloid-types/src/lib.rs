//! Plain data types and rules shared by the Modeloid core, the CLI and the
//! GUI, including the GUI's browser side (so: serde only, no I/O).

pub mod balance;
pub mod classes;
pub mod health;
pub mod split;
pub mod stats;
pub mod validate;

pub use split::Split;
pub use validate::{validate_api_key, validate_dataset_name};
