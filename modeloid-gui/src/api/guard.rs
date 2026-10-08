//! Refuses requests that don't come from Modeloid's own page. The server has
//! the user's rights, and any website the user visits can send requests to
//! 127.0.0.1; browsers mark where a request comes from (`Host`, `Origin`,
//! `Sec-Fetch-Site`) and pages can't change that. Added around every route
//! in `main.rs`.

use dioxus::server::axum::{
    extract::Request,
    http::{header, HeaderMap, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};

const REFUSED: &str = "Refused: the request didn't come from Modeloid's own page.";

pub async fn local_only(req: Request, next: Next) -> Response {
    match check(req.method(), req.uri().path(), req.headers()) {
        Ok(()) => next.run(req).await,
        Err(message) => (StatusCode::FORBIDDEN, message).into_response(),
    }
}

/// Whether a request may reach the app:
/// - `Host` names this computer (a DNS-rebound site names itself);
/// - a request that can change something comes from a page on this computer,
///   when the browser says where it comes from (`Origin`);
/// - the API and dataset files aren't loaded from another site.
///
/// Any port: in development `dx serve` forwards to the app on another port.
fn check(method: &Method, path: &str, headers: &HeaderMap) -> Result<(), &'static str> {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());

    if !text("host").is_some_and(loopback) {
        return Err(REFUSED);
    }
    if !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        if let Some(origin) = headers.get(header::ORIGIN) {
            let local = origin.to_str().ok().and_then(|o| o.strip_prefix("http://")).is_some_and(loopback);
            if !local {
                return Err(REFUSED);
            }
        }
    }
    let protected = path.starts_with("/api/") || path.starts_with("/files/");
    if protected && matches!(text("sec-fetch-site"), Some("cross-site" | "same-site")) {
        return Err(REFUSED);
    }
    Ok(())
}

/// Whether `host` (with an optional port) is this computer.
fn loopback(host: &str) -> bool {
    // "[::1]:8080" → ("::1", ":8080"); "localhost:8080" → ("localhost", ":8080").
    let (name, rest) = match host.strip_prefix('[') {
        Some(inner) => match inner.split_once(']') {
            Some(split) => split,
            None => return false,
        },
        None => host.find(':').map_or((host, ""), |i| host.split_at(i)),
    };
    let port_ok = rest.is_empty() || rest.strip_prefix(':').is_some_and(|p| p.parse::<u16>().is_ok());
    port_ok && (name.eq_ignore_ascii_case("localhost") || name == "127.0.0.1" || name == "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    }

    fn post(pairs: &[(&'static str, &'static str)]) -> Result<(), &'static str> {
        check(&Method::POST, "/api/datasets/delete", &headers(pairs))
    }

    #[test]
    fn own_page_passes() {
        assert!(post(&[("host", "127.0.0.1:8080"), ("origin", "http://127.0.0.1:8080"), ("sec-fetch-site", "same-origin")]).is_ok());
        assert!(post(&[("host", "localhost:8080"), ("origin", "http://localhost:8080")]).is_ok());
        assert!(post(&[("host", "[::1]:8080"), ("origin", "http://[::1]:8080")]).is_ok());
        // dx serve's proxy forwards to the app on another port.
        assert!(post(&[("host", "127.0.0.1:54321"), ("origin", "http://127.0.0.1:8080")]).is_ok());
        // Typed address, bookmark: no site.
        assert!(check(&Method::GET, "/api/datasets", &headers(&[("host", "127.0.0.1:8080"), ("sec-fetch-site", "none")])).is_ok());
    }

    #[test]
    fn local_programs_pass() {
        assert!(post(&[("host", "127.0.0.1:8080")]).is_ok());
    }

    #[test]
    fn other_sites_are_refused() {
        assert!(post(&[("host", "127.0.0.1:8080"), ("origin", "https://evil.com")]).is_err());
        assert!(post(&[("host", "127.0.0.1:8080"), ("origin", "null")]).is_err());
        assert!(post(&[("host", "127.0.0.1:8080"), ("origin", "https://127.0.0.1:8080")]).is_err());
        assert!(post(&[("host", "127.0.0.1:8080"), ("origin", "http://localhost.evil.com")]).is_err());
        assert!(post(&[("host", "127.0.0.1:8080"), ("origin", "http://127.0.0.1.evil.com:8080")]).is_err());
    }

    #[test]
    fn rebound_hosts_are_refused() {
        assert!(post(&[("host", "evil.com:8080"), ("origin", "http://evil.com:8080")]).is_err());
        assert!(check(&Method::GET, "/", &headers(&[("host", "evil.com:8080")])).is_err());
        assert!(check(&Method::GET, "/", &headers(&[("host", "127.0.0.1.evil.com")])).is_err());
        assert!(check(&Method::GET, "/", &headers(&[("host", "[::1].evil.com")])).is_err());
        assert!(check(&Method::GET, "/", &HeaderMap::new()).is_err());
    }

    #[test]
    fn api_and_files_not_loaded_by_other_sites() {
        let from = |site| headers(&[("host", "127.0.0.1:8080"), ("sec-fetch-site", site)]);
        assert!(check(&Method::GET, "/api/datasets", &from("cross-site")).is_err());
        assert!(check(&Method::GET, "/files/image/1/0", &from("same-site")).is_err());
        assert!(check(&Method::GET, "/files/image/1/0", &from("same-origin")).is_ok());
        // Opening the app from a link elsewhere still works.
        assert!(check(&Method::GET, "/", &from("cross-site")).is_ok());
    }
}
