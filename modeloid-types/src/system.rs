//! System settings: how much of this computer Modeloid uses. Stored with the
//! user settings (they describe the machine, not a workspace).

use serde::{Deserialize, Serialize};

/// Most parallel connections a large download may use.
pub const MAX_DOWNLOAD_CONNECTIONS: u32 = 16;
/// Most jobs that may run at the same time.
pub const MAX_JOBS: u32 = 4;
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
    /// These settings within their limits on a computer with `cores` CPU
    /// cores; values a hand-edited file could hold are pulled back in range.
    pub fn normalized(self, cores: u32) -> Self {
        SystemPrefs {
            threads: self.threads.min(cores.max(1)),
            download_connections: self.download_connections.clamp(1, MAX_DOWNLOAD_CONNECTIONS),
            max_jobs: self.max_jobs.clamp(1, MAX_JOBS),
            thumb_size: if THUMB_SIZES.contains(&self.thumb_size) { self.thumb_size } else { SystemPrefs::default().thumb_size },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_values_into_range() {
        let wild = SystemPrefs { threads: 64, download_connections: 0, max_jobs: 99, thumb_size: 1 };
        assert_eq!(wild.normalized(8), SystemPrefs { threads: 8, download_connections: 1, max_jobs: MAX_JOBS, thumb_size: 320 });
        assert_eq!(SystemPrefs::default().normalized(8), SystemPrefs::default());
    }
}
