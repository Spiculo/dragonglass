//! Update checks: compare the running version against one advertised by a source.
//!
//! Dragon Glass never updates itself and never touches the network unless the user points
//! `DRAGON_GLASS_UPDATE_URL` at a source. That source may be a local file path or a plain
//! `http://` URL; HTTPS is not attempted (the version file is not sensitive, and adding a
//! TLS stack for this is not worth it).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// Parse `1.2.3`, tolerating a leading `v` and any `-pre`/`+build` suffix.
pub fn parse(input: &str) -> Option<Version> {
    let core = input.trim().trim_start_matches('v');
    let core = core.split(['-', '+']).next()?;

    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);

    Some(Version {
        major,
        minor,
        patch,
    })
}

pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse(current), parse(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// Read the advertised version from a file path or a plain `http://` URL.
pub fn fetch(source: &str) -> Result<String> {
    match source.strip_prefix("http://") {
        Some(rest) => http_get(rest),
        None => Ok(std::fs::read_to_string(source)?.trim().to_string()),
    }
}

/// `Some(latest)` when the source advertises a version newer than `current`.
pub fn latest_if_newer(current: &str, source: &str) -> Result<Option<String>> {
    let latest = fetch(source)?;
    Ok(if is_newer(current, &latest) {
        Some(latest)
    } else {
        None
    })
}

fn http_get(host_and_path: &str) -> Result<String> {
    let (host, path) = match host_and_path.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (host_and_path, "/".to_string()),
    };
    let addr = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };

    let mut stream = TcpStream::connect(&addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let request = format!("GET {path} HTTP/1.0\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes())?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    Ok(response
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("")
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(
            parse("1.2.3"),
            Some(Version {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(
            parse("v2.0"),
            Some(Version {
                major: 2,
                minor: 0,
                patch: 0
            })
        );
        assert_eq!(
            parse("0.1.0-rc.2"),
            Some(Version {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
        assert_eq!(parse("not-a-version"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.0.1", "0.0.2"));
        assert!(is_newer("0.0.1", "0.1.0"));
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("0.0.2", "0.0.2"));
        assert!(!is_newer("1.0.0", "0.9.9"));
    }

    #[test]
    fn reads_from_a_file() {
        let path = std::env::temp_dir().join(format!(
            "dragon-glass-update-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, "9.9.9\n").unwrap();

        let found = latest_if_newer("0.0.1", path.to_str().unwrap()).unwrap();
        assert_eq!(found.as_deref(), Some("9.9.9"));

        let found = latest_if_newer("99.0.0", path.to_str().unwrap()).unwrap();
        assert_eq!(found, None);

        std::fs::remove_file(&path).unwrap();
    }
}
