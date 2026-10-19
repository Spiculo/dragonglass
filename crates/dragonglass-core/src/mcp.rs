//! A Model Context Protocol server over stdio.
//!
//! Implements the small slice agents need: `initialize`, `tools/list` and `tools/call`,
//! exposing the same search/query/read primitives as the HTTP API. Read-only.

use std::path::Path;

use serde_json::{json, Value};

use crate::{query, search, Vault};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Handle one JSON-RPC message. Returns `None` for notifications (no response expected).
pub fn handle(root: &Path, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(|value| value.as_str())?;

    // Notifications carry no id and get no reply.
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "dragon-glass", "version": env!("CARGO_PKG_VERSION") },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions() })),
        "tools/call" => call_tool(root, request.get("params")),
        other => Err((-32601, format!("method not found: {other}"))),
    };

    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message },
        }),
    })
}

fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "search_notes",
            "description": "Case-insensitive full-text search across the vault.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
            },
        }),
        json!({
            "name": "read_note",
            "description": "Read a note by its vault-relative path.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"],
            },
        }),
        json!({
            "name": "list_notes",
            "description": "List every note in the vault.",
            "inputSchema": { "type": "object", "properties": {} },
        }),
        json!({
            "name": "query_notes",
            "description": "Filter notes by frontmatter, e.g. 'where status done sort by title'.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
            },
        }),
    ]
}

fn call_tool(root: &Path, params: Option<&Value>) -> Result<Value, (i32, String)> {
    let params = params.ok_or((-32602, "missing params".to_string()))?;
    let name = params
        .get("name")
        .and_then(|value| value.as_str())
        .ok_or((-32602, "missing tool name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let vault = Vault::open(root).map_err(|error| (-32603, error.to_string()))?;
    let notes = vault.notes();

    let text = match name {
        "list_notes" => notes
            .iter()
            .map(|note| note.relative.clone())
            .collect::<Vec<_>>()
            .join("\n"),
        "search_notes" => {
            let needle = string_arg(&arguments, "query")?;
            search::full_text(&notes, &needle, 100)
                .into_iter()
                .map(|hit| format!("{}:{}  {}", hit.relative, hit.line_number, hit.line))
                .collect::<Vec<_>>()
                .join("\n")
        }
        "query_notes" => {
            let expression = string_arg(&arguments, "query")?;
            query::run(&notes, &expression)
                .into_iter()
                .map(|row| {
                    let properties = row
                        .values
                        .iter()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>()
                        .join("  ");
                    format!("{}  {properties}", row.relative)
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        "read_note" => {
            let relative = string_arg(&arguments, "path")?;
            let note = notes
                .iter()
                .find(|note| note.relative == relative)
                .ok_or((-32602, format!("note not found: {relative}")))?;
            std::fs::read_to_string(&note.path).map_err(|error| (-32603, error.to_string()))?
        }
        other => return Err((-32602, format!("unknown tool: {other}"))),
    };

    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}

fn string_arg(arguments: &Value, key: &str) -> Result<String, (i32, String)> {
    arguments
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or((-32602, format!("missing `{key}` argument")))
}

/// Read newline-delimited JSON-RPC from stdin and answer on stdout.
pub fn serve(root: &Path) -> std::io::Result<()> {
    use std::io::{BufRead, Write};

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };

        if let Some(response) = handle(root, &request) {
            writeln!(out, "{response}")?;
            out.flush()?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn vault(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dragon-glass-mcp-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.md"), "---\nstatus: done\n---\nhello world\n").unwrap();
        dir
    }

    #[test]
    fn initialize_reports_capabilities() {
        let root = vault("init");
        let response = handle(
            &root,
            &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
        )
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(response["result"]["serverInfo"]["name"], "dragon-glass");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn notifications_get_no_reply() {
        let root = vault("notify");
        assert!(handle(
            &root,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn tools_are_advertised_and_callable() {
        let root = vault("tools");

        let list = handle(
            &root,
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"search_notes"));
        assert!(names.contains(&"read_note"));

        let call = handle(
            &root,
            &json!({
                "jsonrpc":"2.0","id":3,"method":"tools/call",
                "params": {"name":"search_notes","arguments":{"query":"world"}}
            }),
        )
        .unwrap();
        assert!(call["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("hello world"));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unknown_methods_and_tools_error() {
        let root = vault("errors");
        let response = handle(&root, &json!({"jsonrpc":"2.0","id":4,"method":"nope"})).unwrap();
        assert_eq!(response["error"]["code"], -32601);

        let response = handle(
            &root,
            &json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nope","arguments":{}}}),
        )
        .unwrap();
        assert_eq!(response["error"]["code"], -32602);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
