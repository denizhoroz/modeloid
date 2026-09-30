use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed},
    },
    thread,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, future::try_join_all};
use reqwest::{StatusCode, header};
use tokio::sync::mpsc;
use zip::ZipArchive;

use crate::core::limits;
use crate::core::progress::{Progress, Reporter, Unit, check_cancel};

/// Files at least this large are fetched over several connections when the server supports ranges.
const PARALLEL_THRESHOLD: u64 = 16 * 1024 * 1024;
const WRITE_BUF: usize = 1024 * 1024;
const FILE_BUF: usize = 256 * 1024;
const CHANNEL_CAP: usize = 64;

/// How long a rename or delete keeps trying while another program holds the files.
const BUSY_PATIENCE: Duration = Duration::from_secs(10);

/// "In use by another program". On Windows a file or folder can't be renamed
/// or deleted while anything has it (or a file inside it) open, and virus
/// scanners and the search indexer open freshly written files for a moment.
fn is_busy(error: &io::Error) -> bool {
    // 32, 33: sharing and lock violations.
    cfg!(windows) && (error.kind() == io::ErrorKind::PermissionDenied || matches!(error.raw_os_error(), Some(32 | 33)))
}

/// Runs `op` (a rename or delete of `path`), trying again for a while when
/// the files are in use by another program. Still busy after that: an error
/// that says so, instead of a bare "Access is denied".
pub(crate) fn retry_busy<T>(path: &Path, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let started = Instant::now();
    let mut wait = Duration::from_millis(20);
    loop {
        match op() {
            Err(e) if is_busy(&e) && started.elapsed() < BUSY_PATIENCE => {
                thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_millis(500));
            }
            Err(e) if is_busy(&e) => {
                let message = format!(
                    "{} is in use by another program or can't be changed ({e}). Close File Explorer windows, image viewers or editors showing it, then try again",
                    path.display()
                );
                return Err(io::Error::new(e.kind(), message));
            }
            result => return result,
        }
    }
}

/// `fs::rename` that waits out other programs holding the files (see `retry_busy`).
pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    retry_busy(from, || fs::rename(from, to))
}

/// Downloads `url` to `dest` through `dest.part`, returning the number of bytes written.
///
/// Network reads stay on the async runtime while disk writes run on dedicated blocking
/// threads, and large files are split across parallel range requests.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    reporter: &dyn Reporter,
) -> Result<u64, crate::Error> {
    let progress = reporter.start("Downloading", Unit::Bytes);
    let result = download_into(client, url, dest, progress.as_ref()).await;
    progress.finish();
    result
}

/// Like `download`, but counts bytes on `progress` instead of starting its own task.
pub async fn download_into(client: &reqwest::Client, url: &str, dest: &Path, progress: &dyn Progress) -> Result<u64, crate::Error> {
    let part = dest.with_extension("part");
    let result = download_to(client, url, &part, progress).await;
    match result {
        Ok(size) => {
            // A virus scanner may still be reading the finished file.
            let (from, to) = (part.clone(), dest.to_path_buf());
            tokio::task::spawn_blocking(move || rename(&from, &to)).await??;
            Ok(size)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

async fn download_to(client: &reqwest::Client, url: &str, part: &Path, progress: &dyn Progress) -> Result<u64, crate::Error> {
    // Probe with a 1-byte range: a 206 reveals the size and range support, a 200 is the full body.
    let probe = client
        .get(url)
        .header(header::RANGE, "bytes=0-0")
        .send()
        .await?
        .error_for_status()?;

    if probe.status() == StatusCode::PARTIAL_CONTENT {
        let total = content_range_total(probe.headers());
        drop(probe);
        if let Some(total) = total.filter(|&t| t >= PARALLEL_THRESHOLD) {
            progress.set_total(total);
            return download_ranged(client, url, part, total, progress).await;
        }
        let resp = client.get(url).send().await?.error_for_status()?;
        let total = resp.content_length();
        if let Some(t) = total {
            progress.set_total(t);
        }
        return stream_to_file(resp, File::create(part)?, total, progress).await;
    }

    let total = probe.content_length();
    if let Some(t) = total {
        progress.set_total(t);
    }
    stream_to_file(probe, File::create(part)?, total, progress).await
}

async fn download_ranged(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    total: u64,
    progress: &dyn Progress,
) -> Result<u64, crate::Error> {
    File::create(part)?.set_len(total)?;

    let connections = limits::download_connections();
    let parts = split_ranges(total, connections).into_iter().map(|(start, end)| async move {
        let resp = client
            .get(url)
            .header(header::RANGE, format!("bytes={start}-{end}"))
            .send()
            .await?
            .error_for_status()?;
        if resp.status() != StatusCode::PARTIAL_CONTENT {
            return Err(format!("server ignored range request for bytes {start}-{end}").into());
        }

        // Each part gets its own handle: cloned handles would share one file cursor.
        let mut file = OpenOptions::new().write(true).open(part)?;
        file.seek(SeekFrom::Start(start))?;
        stream_to_file(resp, file, Some(end - start + 1), progress).await
    });

    let sizes = try_join_all(parts).await?;
    Ok(sizes.iter().sum())
}

/// Streams a response body into `file`, handing chunks to a blocking writer thread.
async fn stream_to_file(
    resp: reqwest::Response,
    file: File,
    expected: Option<u64>,
    progress: &dyn Progress,
) -> Result<u64, crate::Error> {
    let (tx, rx) = mpsc::channel(CHANNEL_CAP);
    let writer = tokio::task::spawn_blocking(move || write_chunks(rx, file));

    let mut received: u64 = 0;
    let mut stream = resp.bytes_stream();
    let network: Result<(), crate::Error> = async {
        while let Some(chunk) = stream.next().await {
            check_cancel(progress.is_cancelled())?;
            let chunk = chunk?;
            received += chunk.len() as u64;
            progress.add(chunk.len() as u64);
            if tx.send(chunk).await.is_err() {
                break; // writer failed, its error is reported below
            }
        }
        Ok(())
    }
    .await;

    drop(tx);
    let written = writer.await?;
    network?;
    written?;

    match expected {
        Some(n) if n != received => Err(format!("incomplete download: got {received} of {n} bytes").into()),
        _ => Ok(received),
    }
}

fn write_chunks<B: AsRef<[u8]>>(mut rx: mpsc::Receiver<B>, file: File) -> io::Result<()> {
    let mut out = BufWriter::with_capacity(WRITE_BUF, file);
    while let Some(chunk) = rx.blocking_recv() {
        out.write_all(chunk.as_ref())?;
    }
    out.flush()
}

/// Extracts `archive` into `out` using one worker thread per CPU core.
pub async fn extract_zip(archive: &Path, out: &Path, reporter: &dyn Reporter) -> Result<(), crate::Error> {
    let (archive, out) = (archive.to_path_buf(), out.to_path_buf());
    let progress = reporter.start("Extracting", Unit::Files);
    tokio::task::spawn_blocking(move || {
        let result = extract_zip_blocking(&archive, &out, progress.as_ref());
        progress.finish();
        result
    })
        .await?
}

pub(crate) fn extract_zip_blocking(archive: &Path, out: &Path, progress: &dyn Progress) -> Result<(), crate::Error> {
    // Create the whole directory tree up front so workers only ever create files.
    let mut zip = ZipArchive::new(File::open(archive)?)?;
    let mut dirs = HashSet::new();
    let mut files = Vec::new();
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index)?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| format!("unsafe path in archive: {}", entry.name()))?;
        let path = out.join(rel);

        if entry.is_dir() {
            create_dir_once(&mut dirs, &path)?;
        } else if !entry.is_symlink() {
            if let Some(parent) = path.parent() {
                create_dir_once(&mut dirs, parent)?;
            }
            files.push((index, path));
        }
    }
    drop(zip);

    progress.set_total(files.len() as u64);
    run_parallel(
        &files,
        progress,
        || Ok(ZipArchive::new(File::open(archive)?)?),
        |zip, (index, path)| {
            let mut entry = zip.by_index(*index)?;
            let mut out = BufWriter::with_capacity(buf_size(entry.size()), File::create(path)?);
            io::copy(&mut entry, &mut out)?;
            out.flush()?;
            Ok(())
        },
    )
}

/// Moves a directory, creating the destination's parent and copying across volumes if needed.
pub async fn move_dir(src: &Path, dst: &Path, reporter: Arc<dyn Reporter>) -> Result<(), crate::Error> {
    let (src, dst) = (src.to_path_buf(), dst.to_path_buf());
    tokio::task::spawn_blocking(move || move_dir_blocking(&src, &dst, reporter.as_ref()))
        .await?
}

pub(crate) fn move_dir_blocking(src: &Path, dst: &Path, reporter: &dyn Reporter) -> Result<(), crate::Error> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            // Only a cross-volume move does real work, so only it gets a progress bar.
            let progress = reporter.start("Copying", Unit::Files);
            let result = copy_dir_blocking(src, dst, progress.as_ref());
            progress.finish();
            result?;
            retry_busy(src, || fs::remove_dir_all(src))?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn copy_dir_blocking(src: &Path, dst: &Path, progress: &dyn Progress) -> Result<(), crate::Error> {
    let mut files = Vec::new();
    let mut pending = vec![src.to_path_buf()];
    while let Some(dir) = pending.pop() {
        fs::create_dir_all(dst.join(dir.strip_prefix(src)?))?;
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let target = dst.join(path.strip_prefix(src)?);
                files.push((path, target));
            }
        }
    }

    progress.set_total(files.len() as u64);
    run_parallel(&files, progress, || Ok(()), |_, (from, to)| {
        fs::copy(from, to)?;
        Ok(())
    })
}

/// Runs `work` over `items` on the worker threads the system settings allow (one per core by default). Threads pull the next item from a shared
/// counter, so large and small items balance out; the first error stops all workers.
pub(crate) fn run_parallel<T, S>(
    items: &[T],
    progress: &dyn Progress,
    init: impl Fn() -> Result<S, crate::Error> + Sync,
    work: impl Fn(&mut S, &T) -> Result<(), crate::Error> + Sync,
) -> Result<(), crate::Error>
where
    T: Sync,
{
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let first_error: Mutex<Option<crate::Error>> = Mutex::new(None);

    let worker = || -> Result<(), crate::Error> {
        let mut state = init()?;
        while !failed.load(Relaxed) {
            check_cancel(progress.is_cancelled())?;
            let Some(item) = items.get(next.fetch_add(1, Relaxed)) else { break };
            work(&mut state, item)?;
            progress.add(1);
        }
        Ok(())
    };

    thread::scope(|scope| {
        for _ in 0..parallelism().min(items.len()) {
            scope.spawn(|| {
                if let Err(e) = worker() {
                    failed.store(true, Relaxed);
                    first_error.lock().unwrap().get_or_insert(e);
                }
            });
        }
    });

    match first_error.into_inner().unwrap() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn create_dir_once(created: &mut HashSet<PathBuf>, dir: &Path) -> io::Result<()> {
    if !created.contains(dir) {
        fs::create_dir_all(dir)?;
        created.insert(dir.to_path_buf());
    }
    Ok(())
}

/// Splits `0..total` into `parts` contiguous inclusive byte ranges.
fn split_ranges(total: u64, parts: u64) -> Vec<(u64, u64)> {
    debug_assert!(total > 0);
    let parts = parts.clamp(1, total);
    let (base, extra) = (total / parts, total % parts);
    let mut start = 0;
    (0..parts)
        .map(|i| {
            let len = base + u64::from(i < extra);
            let range = (start, start + len - 1);
            start += len;
            range
        })
        .collect()
}

// Content-Range looks like "bytes 0-0/12345"; the total may be "*" when unknown.
fn content_range_total(headers: &header::HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .parse()
        .ok()
}

fn buf_size(len: u64) -> usize {
    usize::try_from(len).map_or(FILE_BUF, |len| len.clamp(1, FILE_BUF))
}

/// Whether `path` starts with a zip signature.
pub fn is_zip(path: &Path) -> io::Result<bool> {
    let mut magic = [0u8; 4];
    let mut file = File::open(path)?;
    let read = io::Read::read(&mut file, &mut magic)?;
    Ok(read == 4 && (magic == *b"PK\x03\x04" || magic == *b"PK\x05\x06"))
}

/// Archives often wrap everything in one folder (`cats/train/...`). If `dir`
/// holds just one folder (ignoring macOS `__MACOSX` junk), moves its contents up.
pub fn unwrap_single_dir(dir: &Path) -> io::Result<()> {
    let _ = fs::remove_dir_all(dir.join("__MACOSX"));
    let entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    let [only] = entries.as_slice() else { return Ok(()) };
    if !only.file_type()?.is_dir() {
        return Ok(());
    }
    // Renamed first, in case it contains an entry with its own name.
    let inner = dir.join(".modeloid-unwrap");
    rename(&only.path(), &inner)?;
    for entry in fs::read_dir(&inner)? {
        let entry = entry?;
        rename(&entry.path(), &dir.join(entry.file_name()))?;
    }
    retry_busy(&inner, || fs::remove_dir(&inner))
}

fn parallelism() -> usize {
    limits::threads()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::progress::Silent;
    use zip::{ZipWriter, write::SimpleFileOptions};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("modeloid-io-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        for (name, data) in entries {
            zip.start_file(name.as_str(), SimpleFileOptions::default()).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn rename_waits_for_a_busy_folder() {
        let tmp = TempDir::new("busy");
        let (from, to) = (tmp.0.join("from"), tmp.0.join("to"));
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join("a.txt"), "x").unwrap();

        // Another program has a file in the folder open for a moment (a virus scan).
        let held = File::open(from.join("a.txt")).unwrap();
        #[cfg(windows)]
        assert!(fs::rename(&from, &to).is_err(), "Windows refuses to rename a folder with an open file");
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        rename(&from, &to).unwrap();
        release.join().unwrap();
        assert!(to.join("a.txt").is_file() && !from.exists());

        // A plain mistake isn't retried.
        let started = Instant::now();
        assert!(rename(&tmp.0.join("missing"), &tmp.0.join("x")).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn extracts_nested_files() {
        let tmp = TempDir::new("extract");
        let entries: Vec<(String, Vec<u8>)> = (0..500)
            .map(|i| (format!("split{}/labels/{i}.txt", i % 3), format!("file {i}").repeat(i).into_bytes()))
            .collect();
        let archive = tmp.0.join("data.zip");
        write_zip(&archive, &entries);

        let out = tmp.0.join("out");
        extract_zip_blocking(&archive, &out, &Silent).unwrap();

        for (name, data) in &entries {
            assert_eq!(&fs::read(out.join(name)).unwrap(), data, "{name}");
        }
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = TempDir::new("zipslip");
        let archive = tmp.0.join("evil.zip");
        write_zip(&archive, &[("../evil.txt".into(), b"x".to_vec())]);

        assert!(extract_zip_blocking(&archive, &tmp.0.join("out"), &Silent).is_err());
        assert!(!tmp.0.join("evil.txt").exists());
    }

    #[test]
    fn splits_ranges_without_gaps() {
        for (total, parts) in [(100, 8), (7, 8), (1, 4), (16 * 1024 * 1024 + 3, 6)] {
            let ranges = split_ranges(total, parts);
            assert_eq!(ranges.first().unwrap().0, 0);
            assert_eq!(ranges.last().unwrap().1, total - 1);
            for pair in ranges.windows(2) {
                assert_eq!(pair[0].1 + 1, pair[1].0);
            }
        }
    }

    #[test]
    fn parses_content_range() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, "bytes 0-0/12345".parse().unwrap());
        assert_eq!(content_range_total(&headers), Some(12345));
        headers.insert(header::CONTENT_RANGE, "bytes 0-0/*".parse().unwrap());
        assert_eq!(content_range_total(&headers), None);
    }

    #[test]
    fn moves_and_copies_dirs() {
        let tmp = TempDir::new("move");
        let src = tmp.0.join("src");
        fs::create_dir_all(src.join("a/b")).unwrap();
        fs::write(src.join("a/b/f.txt"), "hi").unwrap();
        fs::write(src.join("top.txt"), "top").unwrap();

        let copied = tmp.0.join("copy");
        copy_dir_blocking(&src, &copied, &Silent).unwrap();
        assert_eq!(fs::read_to_string(copied.join("a/b/f.txt")).unwrap(), "hi");
        assert_eq!(fs::read_to_string(copied.join("top.txt")).unwrap(), "top");

        let moved = tmp.0.join("missing/parent/dst");
        move_dir_blocking(&src, &moved, &Silent).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(moved.join("a/b/f.txt")).unwrap(), "hi");
    }
}
