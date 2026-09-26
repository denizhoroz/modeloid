use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use modeloid::{Progress, Reporter, Unit};

const TICK: Duration = Duration::from_millis(100);
const TICK_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ";

/// Draws library progress as indicatif bars on stderr. Bars hide themselves when
/// stderr is not a terminal, so piped output stays clean.
pub struct TermReporter {
    bars: MultiProgress,
}

impl TermReporter {
    pub fn new() -> Self {
        TermReporter { bars: MultiProgress::new() }
    }
}

impl Reporter for TermReporter {
    fn start(&self, label: &'static str, unit: Unit) -> Box<dyn Progress> {
        let bar = self.bars.add(ProgressBar::no_length());
        bar.set_style(spinner_style(unit));
        bar.set_prefix(label);
        bar.enable_steady_tick(TICK);
        Box::new(TermBar { bar, unit })
    }

    fn message(&self, msg: &str) {
        // Printing through MultiProgress keeps the line from tearing through active bars.
        if self.bars.is_hidden() || self.bars.println(msg).is_err() {
            eprintln!("{msg}");
        }
    }
}

struct TermBar {
    bar: ProgressBar,
    unit: Unit,
}

impl Progress for TermBar {
    fn set_total(&self, total: u64) {
        self.bar.set_length(total);
        self.bar.set_style(bar_style(self.unit));
    }

    fn add(&self, n: u64) {
        self.bar.inc(n);
    }

    fn finish(&self) {
        self.bar.finish();
    }
}

fn spinner_style(unit: Unit) -> ProgressStyle {
    let counter = match unit {
        Unit::Bytes => "{bytes} ({bytes_per_sec})",
        Unit::Files => "{human_pos} files",
        Unit::Steps => "",
    };
    style(&format!("{{spinner:.cyan}} {{prefix:>11.bold}} {counter}"))
}

fn bar_style(unit: Unit) -> ProgressStyle {
    let counter = match unit {
        Unit::Bytes => "{bytes}/{total_bytes} ({bytes_per_sec}, {eta})",
        Unit::Files => "{human_pos}/{human_len} files ({eta})",
        Unit::Steps => "{pos}/{len}",
    };
    style(&format!("{{spinner:.cyan}} {{prefix:>11.bold}} [{{bar:30.cyan/blue}}] {counter}"))
}

fn style(template: &str) -> ProgressStyle {
    ProgressStyle::with_template(template)
        .expect("progress template is valid")
        .tick_chars(TICK_CHARS)
        .progress_chars("=> ")
}
