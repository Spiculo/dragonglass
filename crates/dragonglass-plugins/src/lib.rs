//! Dragon Glass plugin runtime: manifests, discovery and a sandboxed WASM host.
//!
//! Deliberately not compatible with Obsidian's JavaScript plugins. A Dragon Glass plugin is
//! a WebAssembly module plus a `manifest.json`, so it can be written in any language that
//! targets `wasm32` and runs with no ambient authority.

pub mod host;
pub mod manifest;

pub use host::{run_code, run_command};
pub use manifest::{discover, load, plugins_dir, Command, Manifest, PluginInfo};

/// A plugin's JSON response, including the optional note actions it may request.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct Response {
    #[serde(default)]
    pub message: Option<String>,
    /// Insert this text at the cursor in the active note.
    #[serde(default)]
    pub insert: Option<String>,
    /// Append this text to the end of the active note.
    #[serde(default)]
    pub append: Option<String>,
}

/// Parse a plugin's output, tolerating anything that is not the expected JSON.
pub fn parse_response(output: &str) -> Response {
    serde_json::from_str(output).unwrap_or_default()
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("manifest error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("wasm error: {0}")]
    Wasm(#[from] wasmtime::Error),

    #[error("plugin memory access error: {0}")]
    Memory(#[from] wasmtime::MemoryAccessError),

    #[error("plugin does not export `{0}`")]
    MissingExport(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_messages_and_note_actions() {
        let response = parse_response(r#"{"message":"ok","insert":"fix","append":"tail"}"#);
        assert_eq!(response.message.as_deref(), Some("ok"));
        assert_eq!(response.insert.as_deref(), Some("fix"));
        assert_eq!(response.append.as_deref(), Some("tail"));
    }

    #[test]
    fn non_json_output_is_tolerated() {
        assert_eq!(parse_response("just text"), Response::default());
    }
}
