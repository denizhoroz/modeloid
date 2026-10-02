//! Small formatting helpers shared by pages.

/// Human-readable byte size, e.g. `12.4 MB`.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Number with thousands separators, e.g. `12,345`.
pub fn format_count(n: impl Into<u64>) -> String {
    let digits = n.into().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `1 link`, `3 links`.
pub fn plural(count: usize, word: &str) -> String {
    if count == 1 {
        format!("1 {word}")
    } else {
        format!("{count} {word}s")
    }
}

/// `text` for one URL path segment, or for a path when `keep_slashes`:
/// everything but letters, digits and `-_.~` percent-encoded.
pub fn encode_url_path(text: &str, keep_slashes: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slashes => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn formats_counts() {
        assert_eq!(format_count(7u32), "7");
        assert_eq!(format_count(4267u32), "4,267");
        assert_eq!(format_count(1234567u64), "1,234,567");
    }

    #[test]
    fn pluralizes() {
        assert_eq!(plural(1, "link"), "1 link");
        assert_eq!(plural(0, "link"), "0 links");
    }

    #[test]
    fn encodes_url_paths() {
        assert_eq!(encode_url_path("my run/a b.png", true), "my%20run/a%20b.png");
        assert_eq!(encode_url_path("a/b", false), "a%2Fb");
    }
}
