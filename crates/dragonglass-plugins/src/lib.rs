//! Dragon Glass plugin runtime: manifests, discovery and a sandboxed WASM host.
//!
//! Deliberately not compatible with Obsidian's JavaScript plugins. A Dragon Glass plugin is
//! a WebAssembly module plus a `manifest.json`, so it can be written in any language that
//! targets `wasm32` and runs with no ambient authority.

pub mod host;
pub mod manifest;

pub use host::{run_code, run_command};
pub use manifest::{discover, load, plugins_dir, Command, Manifest, PluginInfo};

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
