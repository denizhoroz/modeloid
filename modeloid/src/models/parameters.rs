//! Training parameter sets: named, reusable model and training settings,
//! saved in the workspace as `models/parameters/<name>.toml` (see
//! `Workspace::parameter_sets`). The types live in `modeloid-types` so the
//! GUI's browser side can edit them too.

pub use modeloid_types::parameters::*;
