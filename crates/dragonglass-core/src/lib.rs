//! Dragon Glass core: the headless engine. No UI dependencies.
//!
//! Everything here is a plain Rust library so it can be unit-tested, driven by the CLI,
//! and reused by the GPUI application.

pub mod buffer;
pub mod frontmatter;
pub mod git;
pub mod markdown;
pub mod note;
pub mod search;
pub mod storage;
pub mod tags;
pub mod templates;
pub mod tree;
pub mod vault;
pub mod wikilink;

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
}

pub type Result<T> = std::result::Result<T, Error>;
