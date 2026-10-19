//! Dragon Glass core: the headless engine. No UI dependencies.
//!
//! Everything here is a plain Rust library so it can be unit-tested, driven by the CLI,
//! and reused by the GPUI application.

pub mod api;
pub mod bookmarks;
pub mod buffer;
pub mod config;
pub mod conflict;
pub mod frontmatter;
pub mod git;
pub mod graph;
pub mod history;
pub mod html;
pub mod import;
pub mod keymap;
pub mod markdown;
pub mod mcp;
pub mod note;
pub mod pdf;
pub mod query;
pub mod search;
pub mod storage;
pub mod store;
pub mod tags;
pub mod templates;
pub mod tree;
pub mod update;
pub mod uri;
pub mod vault;
pub mod wikilink;
pub mod workspace;
pub mod wrap;

pub use buffer::Buffer;
pub use note::Note;
pub use tree::{NodeKind, TreeNode};
pub use vault::{NoteRef, Vault};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a directory: {0}")]
    NotADirectory(std::path::PathBuf),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{command} failed: {message}")]
    Git { command: String, message: String },

    #[error("unsupported frontmatter on line {line}: {text}")]
    UnsupportedFrontmatter { line: usize, text: String },

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
