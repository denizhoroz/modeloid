//! System settings: how much of this computer Modeloid uses. Stored with the
//! user settings (they describe the machine, not a workspace).

use serde::{Deserialize, Serialize};

/// Most parallel connections a large download may use.
pub const MAX_DOWNLOAD_CONNECTIONS: u32 = 32;
/// Most jobs that may run at the same time, on any computer.
pub const MAX_JOBS: u32 = 16;
/// Jobs at once every computer may run: a light one (a download) beside another.
const MIN_MACHINE_JOBS: u32 = 2;
const GIB: u64 = 1 << 30;

/// What a computer has; it caps the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    /// CPU cores (logical).
    pub cores: u32,
    /// Bytes of memory; `None` when the OS doesn't say.
    pub memory: Option<u64>,
}

impl Machine {
    /// Jobs at once this computer can take: one per 2 CPU cores and per
    /// 4 GB of memory, at least `MIN_MACHINE_JOBS`, at most `MAX_JOBS`.
    pub fn max_jobs(&self) -> u32 {
        let by_cores = self.cores / 2;
        // Rounded to the nearest 4 GB: a "32 GB" computer reports a GB or so less.
        let by_memory = self.memory.map_or(u32::MAX, |bytes| ((bytes + 2 * GIB) / (4 * GIB)).min(u32::MAX as u64) as u32);
        by_cores.min(by_memory).clamp(MIN_MACHINE_JOBS, MAX_JOBS)
    }
}
/// Longest side of gallery previews the settings offer, in pixels.
pub const THUMB_SIZES: [u32; 4] = [160, 240, 320, 480];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemPrefs {
    /// Threads for file work (copying, extracting, decoding images); 0 = one per CPU core.
    pub threads: u32,
    /// Parallel connections for one large download.
    pub download_connections: u32,
    /// Jobs that may run at the same time (each a different tool, none
    /// sharing a dataset with another).
    pub max_jobs: u32,
    /// Longest side of gallery previews, in pixels.
    pub thumb_size: u32,
}

impl Default for SystemPrefs {
    fn default() -> Self {
        SystemPrefs { threads: 0, download_connections: 8, max_jobs: 1, thumb_size: 320 }
    }
}

impl SystemPrefs {
    /// These settings within the limits of `machine`; values a hand-edited
    /// file (or one from another computer) could hold are pulled back in range.
    pub fn normalized(self, machine: Machine) -> Self {
        SystemPrefs {
            threads: self.threads.min(machine.cores.max(1)),
            download_connections: self.download_connections.clamp(1, MAX_DOWNLOAD_CONNECTIONS),
            max_jobs: self.max_jobs.clamp(1, machine.max_jobs()),
            thumb_size: if THUMB_SIZES.contains(&self.thumb_size) { self.thumb_size } else { SystemPrefs::default().thumb_size },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_values_into_range() {
        let machine = Machine { cores: 8, memory: Some(32 * GIB) };
        let wild = SystemPrefs { threads: 64, download_connections: 0, max_jobs: 99, thumb_size: 1 };
        assert_eq!(wild.normalized(machine), SystemPrefs { threads: 8, download_connections: 1, max_jobs: 4, thumb_size: 320 });
        assert_eq!(SystemPrefs::default().normalized(machine), SystemPrefs::default());
    }

    #[test]
    fn jobs_follow_the_computer() {
        let jobs = |cores, gb: u64| Machine { cores, memory: Some(gb * GIB) }.max_jobs();
        assert_eq!(jobs(4, 8), 2, "small laptop: the floor");
        assert_eq!(jobs(2, 4), 2);
        assert_eq!(jobs(16, 32), 8);
        assert_eq!(jobs(16, 8), 2, "memory caps it");
        assert_eq!(jobs(64, 256), MAX_JOBS);
        let almost_32 = Machine { cores: 16, memory: Some(33_449_963_520) };
        assert_eq!(almost_32.max_jobs(), 8, "reported memory rounds to the nearest 4 GB");
        assert_eq!(Machine { cores: 12, memory: None }.max_jobs(), 6, "cores alone when memory is unknown");
    }
}
