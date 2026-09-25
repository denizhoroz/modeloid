use std::{
    collections::HashSet,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::Relaxed},
    },
    thread,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, future::try_join_all};
use reqwest::{StatusCode, header};
use tokio::sync::mpsc;
use zip::ZipArchive;

/// Files at least this large are fetched over several connections when the server supports ranges.
const PARALLEL_THRESHOLD: u64 = 16 * 1024 * 1024;
const MIN_CONNECTIONS: usize = 4;
const MAX_CONNECTIONS: usize = 8;
const WRITE_BUF: usize = 1024 * 1024;
const FILE_BUF: usize = 256 * 1024;
const CHANNEL_CAP: usize = 64;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

type SendError = Box<dyn Error + Send + Sync>;

/// Downloads `url` to `dest` through `dest.part`, returning the number of bytes written.
///
/// Network reads stay on the async runtime while disk writes run on dedicated blocking
/// threads, and large files are split across parallel range requests.
pub async fn download(client: &reqwest::Client, url: &str, dest: &Path) -> Result<u64, Box<dyn Error>> {
    let part = dest.with_extension("part");
    let progress = Progress::new("Downloading", Unit::Bytes);

    let result = download_to(client, url, &part, &progress).await;
    progress.finish();

    match result {
        Ok(size) => {
            tokio::fs::rename(&part, dest).await?;
            Ok(size)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

async fn download_to(client: &reqwest::Client, url: &str, part: &Path, progress: &Progress) -> Result<u64, Box<dyn Error>> {
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
    progress: &Progress,
) -> Result<u64, Box<dyn Error>> {
    File::create(part)?.set_len(total)?;

    let connections = parallelism().clamp(MIN_CONNECTIONS, MAX_CONNECTIONS) as u64;
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
    progress: &Progress,
) -> Result<u64, Box<dyn Error>> {
    let (tx, rx) = mpsc::channel(CHANNEL_CAP);
    let writer = tokio::task::spawn_blocking(move || write_chunks(rx, file));

    let mut received: u64 = 0;
    let mut stream = resp.bytes_stream();
    let network: Result<(), Box<dyn Error>> = async {
        while let Some(chunk) = stream.next().await {
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
pub async fn extract_zip(archive: &Path, out: &Path) -> Result<(), Box<dyn Error>> {
    let (archive, out) = (archive.to_path_buf(), out.to_path_buf());
    tokio::task::spawn_blocking(move || extract_zip_blocking(&archive, &out))
        .await?
        .map_err(unsend)
}

fn extract_zip_blocking(archive: &Path, out: &Path) -> Result<(), SendError> {
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

    let progress = Progress::new("Extracting", Unit::Files);
    progress.set_total(files.len() as u64);
    let result = run_parallel(
        &files,
        &progress,
        || Ok(ZipArchive::new(File::open(archive)?)?),
        |zip, (index, path)| {
            let mut entry = zip.by_index(*index)?;
            let mut out = BufWriter::with_capacity(buf_size(entry.size()), File::create(path)?);
            io::copy(&mut entry, &mut out)?;
            out.flush()?;
            Ok(())
        },
    );
    progress.finish();
    result
}

/// Moves a directory, creating the destination's parent and copying across volumes if needed.
pub async fn move_dir(src: &Path, dst: &Path) -> Result<(), Box<dyn Error>> {
    let (src, dst) = (src.to_path_buf(), dst.to_path_buf());
    tokio::task::spawn_blocking(move || move_dir_blocking(&src, &dst))
        .await?
        .map_err(unsend)
}

fn move_dir_blocking(src: &Path, dst: &Path) -> Result<(), SendError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
            copy_dir_blocking(src, dst)?;
            fs::remove_dir_all(src)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn copy_dir_blocking(src: &Path, dst: &Path) -> Result<(), SendError> {
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

    let progress = Progress::new("Copying", Unit::Files);
    progress.set_total(files.len() as u64);
    let result = run_parallel(&files, &progress, || Ok(()), |_, (from, to)| {
        fs::copy(from, to)?;
        Ok(())
    });
    progress.finish();
    result
}

/// Runs `work` over `items` on one thread per core. Threads pull the next item from a shared
/// counter, so large and small items balance out; the first error stops all workers.
fn run_parallel<T, S>(
    items: &[T],
    progress: &Progress,
    init: impl Fn() -> Result<S, SendError> + Sync,
    work: impl Fn(&mut S, &T) -> Result<(), SendError> + Sync,
) -> Result<(), SendError>
where
    T: Sync,
{
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let first_error: Mutex<Option<SendError>> = Mutex::new(None);

    let worker = || -> Result<(), SendError> {
        let mut state = init()?;
        while !failed.load(Relaxed) {
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

fn parallelism() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get())
}

fn unsend(e: SendError) -> Box<dyn Error> {
    e
}

enum Unit {
    Bytes,
    Files,
}

/// Thread-safe progress line on stderr, redrawn at most every `PROGRESS_INTERVAL`.
struct Progress {
    label: &'static str,
    unit: Unit,
    done: AtomicU64,
    total: AtomicU64,
    last_draw: Mutex<Instant>,
}

impl Progress {
    fn new(label: &'static str, unit: Unit) -> Self {
        Progress {
            label,
            unit,
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            last_draw: Mutex::new(Instant::now()),
        }
    }

    fn set_total(&self, total: u64) {
        self.total.store(total, Relaxed);
    }

    fn add(&self, n: u64) {
        let done = self.done.fetch_add(n, Relaxed) + n;
        // try_lock: a busy lock means another thread is drawing right now.
        if let Ok(mut last) = self.last_draw.try_lock()
            && last.elapsed() >= PROGRESS_INTERVAL
        {
            *last = Instant::now();
            self.draw(done);
        }
    }

    fn finish(&self) {
        self.draw(self.done.load(Relaxed));
        eprintln!();
    }

    fn draw(&self, done: u64) {
        let total = self.total.load(Relaxed);
        let mb = |b: u64| b as f64 / 1_048_576.0;
        let pct = |d: u64| d as f64 / total as f64 * 100.0;
        let line = match (&self.unit, total) {
            (Unit::Bytes, 0) => format!("{:.1} MB", mb(done)),
            (Unit::Bytes, _) => format!("{:.1}/{:.1} MB ({:.0}%)", mb(done), mb(total), pct(done)),
            (Unit::Files, 0) => format!("{done} files"),
            (Unit::Files, _) => format!("{done}/{total} files ({:.0}%)", pct(done)),
        };
        eprint!("\r{} {line}", self.label);
        let _ = io::stderr().flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn extracts_nested_files() {
        let tmp = TempDir::new("extract");
        let entries: Vec<(String, Vec<u8>)> = (0..500)
            .map(|i| (format!("split{}/labels/{i}.txt", i % 3), format!("file {i}").repeat(i).into_bytes()))
            .collect();
        let archive = tmp.0.join("data.zip");
        write_zip(&archive, &entries);

        let out = tmp.0.join("out");
        extract_zip_blocking(&archive, &out).unwrap();

        for (name, data) in &entries {
            assert_eq!(&fs::read(out.join(name)).unwrap(), data, "{name}");
        }
    }

    #[test]
    fn rejects_path_traversal() {
        let tmp = TempDir::new("zipslip");
        let archive = tmp.0.join("evil.zip");
        write_zip(&archive, &[("../evil.txt".into(), b"x".to_vec())]);

        assert!(extract_zip_blocking(&archive, &tmp.0.join("out")).is_err());
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
        copy_dir_blocking(&src, &copied).unwrap();
        assert_eq!(fs::read_to_string(copied.join("a/b/f.txt")).unwrap(), "hi");
        assert_eq!(fs::read_to_string(copied.join("top.txt")).unwrap(), "top");

        let moved = tmp.0.join("missing/parent/dst");
        move_dir_blocking(&src, &moved).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read_to_string(moved.join("a/b/f.txt")).unwrap(), "hi");
    }
}
