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
}

/// Creates progress tasks and receives status messages. The library never draws to the
/// terminal itself; frontends such as the CLI decide how progress is shown.
pub trait Reporter: Send + Sync {
    fn start(&self, label: &'static str, unit: Unit) -> Box<dyn Progress>;
    fn message(&self, msg: &str);
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
