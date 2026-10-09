//! Starting Modeloid the way users do (double-clicking the app, not `dx serve`):
//! one copy per user, on a free port, with the browser opened on it once the
//! server listens.
//!
//! `<config dir>/modeloid/instance` holds the running copy's port and a random
//! token, and `/api/instance` answers with that token. A second launch asks the
//! port in the file: the same token means Modeloid is already running, so it
//! only opens the browser and exits. Anything else (no answer, another program
//! on that port, a file left by a closed or crashed run) means start a new
//! server. The app stops when its console window is closed.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use dioxus::cli_config;
use dioxus::server::axum::{routing::get, Router};

/// Tried first when the last run's port is taken or unknown.
const DEFAULT_PORT: u16 = 8080;

/// How long to wait for the server to listen before opening the browser anyway.
const LISTEN_WAIT: Duration = Duration::from_secs(60);

/// This copy's token; unset under `dx serve`.
static TOKEN: OnceLock<String> = OnceLock::new();

#[derive(Debug, PartialEq)]
pub enum Launch {
    /// Start the server.
    Serve,
    /// Another copy is running and the browser was opened on it: exit.
    AlreadyRunning,
}

/// Picks the port and claims the instance file; call before the server starts.
pub fn prepare() -> Launch {
    // `dx serve` picks the address and opens the browser itself.
    if cli_config::is_cli_enabled() {
        return Launch::Serve;
    }

    let file = instance_path();
    let last = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|text| Instance::parse(&text));
    if let Some(running) = &last {
        if ping(running.port).as_deref() == Some(running.token.as_str()) {
            open_browser(&url(running.port));
            return Launch::AlreadyRunning;
        }
    }

    // A port the user set (`PORT=…`) wins; otherwise the last run's, so
    // bookmarks keep working, then the default, then any free one.
    let port = cli_config::server_port().unwrap_or_else(|| {
        let port = free_port(last.as_ref().map(|i| i.port));
        // Read by `dioxus::serve`; nothing else runs yet.
        std::env::set_var(cli_config::SERVER_PORT_ENV, port.to_string());
        port
    });

    let token = new_token();
    if let Some(file) = &file {
        let text = Instance { port, token: token.clone() }.to_text();
        if let Err(e) = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(file, text)) {
            eprintln!("Couldn't write {}: {e}", file.display());
        }
    }
    let _ = TOKEN.set(token);

    std::thread::spawn(move || {
        let ready = wait_for_listen(port);
        let address = url(port);
        println!("Modeloid is running at {address}");
        if ready {
            open_browser(&address);
        }
    });
    Launch::Serve
}

/// `/api/instance`, which tells a second launch this copy is running.
pub fn routes() -> Router {
    Router::new().route("/api/instance", get(|| async { TOKEN.get().cloned().unwrap_or_default() }))
}

fn instance_path() -> Option<PathBuf> {
    modeloid::Settings::default_path().map(|settings| settings.with_file_name("instance"))
}

fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The running copy, as written to the instance file: the port, then the token.
#[derive(Debug, PartialEq)]
struct Instance {
    port: u16,
    token: String,
}

impl Instance {
    fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines().map(str::trim);
        let port = lines.next()?.parse().ok()?;
        let token = lines.next().filter(|t| !t.is_empty())?.to_string();
        Some(Instance { port, token })
    }

    fn to_text(&self) -> String {
        format!("{}\n{}\n", self.port, self.token)
    }
}

/// 32 hex characters. Not a secret (any local program can read the file); it
/// only tells this Modeloid apart from other programs and older runs.
fn new_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let part = || {
        let mut h = RandomState::new().build_hasher();
        h.write_u32(std::process::id());
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

/// The first of `preferred`, `DEFAULT_PORT` that's free on 127.0.0.1, else one
/// the system picks.
fn free_port(preferred: Option<u16>) -> u16 {
    let free = |port: u16| TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok();
    preferred
        .into_iter()
        .chain([DEFAULT_PORT])
        .find(|&port| port != 0 && free(port))
        .or_else(|| TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?.local_addr().ok().map(|a| a.port()))
        .unwrap_or(DEFAULT_PORT)
}

/// The body of `/api/instance` on `port`, if something answers there with 200.
fn ping(port: u16) -> Option<String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok()?;
    let request = format!("GET /api/instance HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).ok()?;
    let mut answer = String::new();
    stream.take(64 * 1024).read_to_string(&mut answer).ok()?;
    response_body(&answer)
}

/// The body of an HTTP/1.1 200 answer (`Connection: close`, not chunked).
fn response_body(answer: &str) -> Option<String> {
    let (head, body) = answer.split_once("\r\n\r\n")?;
    let status = head.lines().next()?;
    status.split_whitespace().nth(1).filter(|code| *code == "200")?;
    Some(body.trim().to_string())
}

/// Waits until something listens on `port`; false after `LISTEN_WAIT`.
fn wait_for_listen(port: u16) -> bool {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let start = Instant::now();
    while start.elapsed() < LISTEN_WAIT {
        if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Opens `url` in the default browser. The address is printed too, for
/// systems without one (or when this fails).
fn open_browser(url: &str) {
    use std::process::Command;

    #[cfg(windows)]
    let result = Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(url).spawn();
    #[cfg(not(any(windows, target_os = "macos")))]
    let result = Command::new("xdg-open").arg(url).spawn();

    if let Err(e) = result {
        eprintln!("Couldn't open the browser ({e}). Open {url} yourself.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_file_round_trips() {
        let instance = Instance { port: 8081, token: "abc123".into() };
        assert_eq!(Instance::parse(&instance.to_text()), Some(instance));
    }

    #[test]
    fn broken_instance_files_are_ignored() {
        assert_eq!(Instance::parse(""), None);
        assert_eq!(Instance::parse("8080\n"), None);
        assert_eq!(Instance::parse("8080\n\n"), None);
        assert_eq!(Instance::parse("port\nabc\n"), None);
        assert_eq!(Instance::parse("70000\nabc\n"), None);
    }

    #[test]
    fn tokens_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }

    #[test]
    fn reads_only_ok_bodies() {
        let ok = "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\n\r\nabc123";
        assert_eq!(response_body(ok).as_deref(), Some("abc123"));
        assert_eq!(response_body("HTTP/1.1 404 Not Found\r\n\r\nabc123"), None);
        assert_eq!(response_body("HTTP/1.1 403 Forbidden\r\n\r\nRefused"), None);
        assert_eq!(response_body("garbage"), None);
    }

    #[test]
    fn taken_port_is_skipped() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        assert_ne!(free_port(Some(port)), port);
    }

    #[test]
    fn ping_reads_the_token() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 1024];
            let _ = stream.read(&mut buf);
            stream.write_all(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\ntoken42").unwrap();
        });
        assert_eq!(ping(port).as_deref(), Some("token42"));
    }
}
