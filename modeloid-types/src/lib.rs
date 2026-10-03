//! Plain data types and rules shared by the Modeloid core and the
//! GUI, including the GUI's browser side (so: serde only, no I/O).

pub mod balance;
pub mod classes;
pub mod convert;
pub mod edit;
pub mod health;
pub mod history;
pub mod parameters;
pub mod pyenv;
pub mod split;
pub mod stats;
pub mod system;
pub mod train;
pub mod tune;
pub mod validate;

pub use split::Split;
pub use validate::{validate_api_key, validate_dataset_name};
