//! How much of this computer the core uses right now: worker threads,
//! download connections, jobs at once, preview size. Process-wide, set once
//! from the user's settings (`Settings::apply_system`) and again when they
//! change; without that the defaults apply.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::thread;

pub use modeloid_types::system::*;

static THREADS: AtomicU32 = AtomicU32::new(0);
static DOWNLOAD_CONNECTIONS: AtomicU32 = AtomicU32::new(8);
static MAX_RUNNING_JOBS: AtomicU32 = AtomicU32::new(1);
static THUMB_SIZE: AtomicU32 = AtomicU32::new(320);

/// CPU cores of this computer (4 when the OS doesn't say).
pub fn cores() -> u32 {
    thread::available_parallelism().map_or(4, |n| n.get() as u32)
}

/// Bytes of memory in this computer; `None` when the OS doesn't say.
pub fn memory() -> Option<u64> {
    total_memory().filter(|&bytes| bytes > 0)
}

/// What this computer has, for the settings' limits.
pub fn machine() -> Machine {
    Machine { cores: cores(), memory: memory() }
}

#[cfg(windows)]
fn total_memory() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: MEMORYSTATUSEX is plain data; the call only writes into it.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullTotalPhys)
}

#[cfg(target_os = "macos")]
fn total_memory() -> Option<u64> {
    let mut bytes = 0u64;
    let mut len = size_of::<u64>();
    // SAFETY: hw.memsize is a u64; `len` tells sysctl the buffer's size.
    let ok = unsafe { libc::sysctlbyname(c"hw.memsize".as_ptr(), (&raw mut bytes).cast(), &mut len, std::ptr::null_mut(), 0) } == 0;
    ok.then_some(bytes)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn total_memory() -> Option<u64> {
    // "MemTotal:       16318412 kB"
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = info.lines().find_map(|line| line.strip_prefix("MemTotal:"))?.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    Some(kb * 1024)
}

/// Makes `prefs` (pulled into range) the limits in use.
pub fn apply(prefs: SystemPrefs) {
    let prefs = prefs.normalized(machine());
    THREADS.store(prefs.threads, Relaxed);
    DOWNLOAD_CONNECTIONS.store(prefs.download_connections, Relaxed);
    MAX_RUNNING_JOBS.store(prefs.max_jobs, Relaxed);
    THUMB_SIZE.store(prefs.thumb_size, Relaxed);
}

/// The limits in use.
pub fn current() -> SystemPrefs {
    SystemPrefs {
        threads: THREADS.load(Relaxed),
        download_connections: DOWNLOAD_CONNECTIONS.load(Relaxed),
        max_jobs: MAX_RUNNING_JOBS.load(Relaxed),
        thumb_size: THUMB_SIZE.load(Relaxed),
    }
}

/// Threads for file work: the configured number, or one per core.
pub fn threads() -> usize {
    match THREADS.load(Relaxed) {
        0 => cores() as usize,
        n => n as usize,
    }
}

pub fn download_connections() -> u64 {
    u64::from(DOWNLOAD_CONNECTIONS.load(Relaxed).max(1))
}

pub fn max_jobs() -> usize {
    MAX_RUNNING_JOBS.load(Relaxed).max(1) as usize
}

pub fn thumb_size() -> u32 {
    THUMB_SIZE.load(Relaxed)
}
