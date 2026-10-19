//! A tiny read-only HTTP API over a vault, for editor tooling and AI agents.
//!
//! Deliberately dependency-free and local-only: bind to loopback and expose a handful of
//! GET endpoints. Anything that opens this port can read notes, which is exactly the
//! intent, so it is not meant to be exposed publicly.
//!
//! ```text
//! GET /health              -> {"status":"ok"}
//! GET /notes               -> {"notes":["a.md", ...]}
//! GET /search?q=needle     -> {"matches":[{"note":..,"line":..,"text":..}]}
//! GET /query?q=where+tag+x -> {"rows":[{"note":..,"title":..,"properties":{..}}]}
//! GET /note?path=a.md      -> raw markdown (403 if it escapes the vault)
//! ```

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;

use crate::{query, search, Vault};

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

pub fn handle(target: &str, root: &Path) -> Response {
    let (path, query_string) = match target.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (target, None),
    };

    let Ok(vault) = Vault::open(root) else {
        return text(500, "cannot open vault");
    };
    let notes = vault.notes();

    match path {
        "/health" => json(200, "{\"status\":\"ok\"}"),
        "/notes" => {
            let list: Vec<String> = notes
                .iter()
                .map(|note| json_string(&note.relative))
                .collect();
            json(200, &format!("{{\"notes\":[{}]}}", list.join(",")))
        }
        "/search" => {
            let needle = param(query_string, "q").unwrap_or_default();
            let matches = search::full_text(&notes, &needle, 100);
            let items: Vec<String> = matches
                .iter()
                .map(|hit| {
                    format!(
                        "{{\"note\":{},\"line\":{},\"text\":{}}}",
                        json_string(&hit.relative),
                        hit.line_number,
                        json_string(&hit.line)
                    )
                })
                .collect();
            json(200, &format!("{{\"matches\":[{}]}}", items.join(",")))
        }
        "/query" => {
            let expression = param(query_string, "q").unwrap_or_default();
            let rows = query::run(&notes, &expression);
            let items: Vec<String> = rows
                .iter()
                .map(|row| {
                    let properties: Vec<String> = row
                        .values
                        .iter()
                        .map(|(key, value)| format!("{}:{}", json_string(key), json_string(value)))
                        .collect();
                    format!(
                        "{{\"note\":{},\"title\":{},\"properties\":{{{}}}}}",
                        json_string(&row.relative),
                        json_string(&row.title),
                        properties.join(",")
                    )
                })
                .collect();
            json(200, &format!("{{\"rows\":[{}]}}", items.join(",")))
        }
        "/note" => {
            let relative = param(query_string, "path").unwrap_or_default();
            match read_within(root, &relative) {
                Ok(content) => Response {
                    status: 200,
                    content_type: "text/markdown",
                    body: content,
                },
                Err(ReadError::Outside) => text(403, "path outside vault"),
                Err(ReadError::Missing) => text(404, "note not found"),
            }
        }
        _ => text(404, "not found"),
    }
}

/// Serve the API until the process is killed.
pub fn serve(root: &Path, addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;

    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };

        let mut buffer = [0u8; 8192];
        let read = stream.read(&mut buffer)?;
        let request = String::from_utf8_lossy(&buffer[..read]);

        let target = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/");

        let response = handle(target, root);
        let head = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.status,
            reason(response.status),
            response.content_type,
            response.body.len()
        );

        stream.write_all(head.as_bytes())?;
        stream.write_all(response.body.as_bytes())?;
    }

    Ok(())
}

enum ReadError {
    Outside,
    Missing,
}

fn read_within(root: &Path, relative: &str) -> Result<String, ReadError> {
    let root = root.canonicalize().map_err(|_| ReadError::Missing)?;
    let candidate = root.join(relative);
    let resolved = candidate.canonicalize().map_err(|_| ReadError::Missing)?;

    if !resolved.starts_with(&root) {
        return Err(ReadError::Outside);
    }
    std::fs::read_to_string(resolved).map_err(|_| ReadError::Missing)
}

fn param(query: Option<&str>, key: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        if let Some((name, value)) = pair.split_once('=') {
            if name == key {
                return Some(percent_decode(value));
            }
        }
    }
    None
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }

    String::from_utf8_lossy(&out).to_string()
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json(status: u16, body: &str) -> Response {
    Response {
        status,
        content_type: "application/json",
        body: body.to_string(),
    }
}

fn text(status: u16, body: &str) -> Response {
    Response {
        status,
        content_type: "text/plain",
        body: body.to_string(),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_vault(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-api-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::write(
            dir.join("notes/a.md"),
            "---\ntitle: Alpha\n---\nhello world\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes/b.md"), "nothing here\n").unwrap();
        dir
    }

    #[test]
    fn health_and_notes() {
        let root = temp_vault("list");
        let response = handle("/health", &root);
        assert_eq!(response.status, 200);
        assert!(response.body.contains("ok"));

        let response = handle("/notes", &root);
        assert!(response.body.contains("notes/a.md"));
        assert!(response.body.contains("notes/b.md"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn search_and_note() {
        let root = temp_vault("search");
        let response = handle("/search?q=world", &root);
        assert!(response.body.contains("hello world"));

        let response = handle("/note?path=notes%2Fa.md", &root);
        assert_eq!(response.status, 200);
        assert!(response.body.contains("hello world"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn query_endpoint_reads_properties() {
        let root = temp_vault("query");
        let response = handle("/query?q=where+title+Alpha", &root);
        assert!(response.body.contains("Alpha"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn rejects_escaping_paths_and_unknown_routes() {
        let container = std::env::temp_dir().join(format!(
            "dragon-glass-api-safe-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&container);

        let vault = container.join("vault");
        std::fs::create_dir_all(vault.join("notes")).unwrap();
        std::fs::write(vault.join("notes/a.md"), "hi\n").unwrap();
        // A file that exists, but outside the vault.
        std::fs::write(container.join("secret.md"), "secret\n").unwrap();

        assert_eq!(handle("/note?path=../secret.md", &vault).status, 403);
        assert_eq!(handle("/nope", &vault).status, 404);

        std::fs::remove_dir_all(&container).unwrap();
    }
}
