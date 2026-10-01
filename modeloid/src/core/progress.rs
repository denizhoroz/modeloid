/// What a progress counter counts, so frontends can format it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Bytes,
    Files,
    Steps,
}

/// One running task. A task without a total is shown as indeterminate.
pub trait Progress: Send + Sync {
    fn set_total(&self, total: u64);
    fn add(&self, n: u64);
    fn finish(&self);

    /// True once the user asked to stop. Long loops check it and return `Cancelled`.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Creates progress tasks and receives status messages. The library never draws to the
/// terminal itself; the frontend (the GUI's job runner) decides how progress is shown.
pub trait Reporter: Send + Sync {
    fn start(&self, label: &'static str, unit: Unit) -> Box<dyn Progress>;
    fn message(&self, msg: &str);

    /// True once the user asked to stop; see `Progress::is_cancelled`.
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Error returned when work stops because the user cancelled it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// `Err(Cancelled)` if `cancelled`, for `?` in loops.
pub fn check_cancel(cancelled: bool) -> Result<(), Cancelled> {
    if cancelled { Err(Cancelled) } else { Ok(()) }
}

/// Reporter that discards everything.
pub struct Silent;

impl Reporter for Silent {
    fn start(&self, _label: &'static str, _unit: Unit) -> Box<dyn Progress> {
        Box::new(Silent)
    }

    fn message(&self, _msg: &str) {}
}

impl Progress for Silent {
    fn set_total(&self, _total: u64) {}
    fn add(&self, _n: u64) {}
    fn finish(&self) {}
}
