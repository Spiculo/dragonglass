//! `dragon-glass` — headless CLI over a vault.
//!
//! Usage:
//!   dragon-glass stats  [vault]
//!   dragon-glass broken [vault]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use dragonglass_core::{Note, NoteRef, Vault};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (command, path) = match args.next().as_deref() {
        None | Some("-h") | Some("--help") => {
            print_help();
            return Ok(());
        }
        Some("stats") => ("stats", args.next()),
        Some("broken") => ("broken", args.next()),
        Some(other) => ("stats", Some(other.to_string())),
    };

    let root = PathBuf::from(path.unwrap_or_else(|| ".".to_string()));
    let vault = Vault::open(&root).with_context(|| format!("opening vault {}", root.display()))?;
    let refs = vault.notes();

    let mut loaded: Vec<(NoteRef, Note)> = Vec::with_capacity(refs.len());
    let mut read_errors = 0usize;
    for r in refs {
        match Note::load(&r.path) {
            Ok(note) => loaded.push((r, note)),
            Err(_) => read_errors += 1,
        }
    }

    let resolver = Resolver::new(loaded.iter().map(|(r, _)| r));

    let mut total_links = 0usize;
    let mut broken = 0usize;
    let mut ambiguous = 0usize;
    let mut attachments = 0usize;
    let mut unique_targets: HashMap<String, usize> = HashMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    let mut broken_list: Vec<(String, String)> = Vec::new();

    for (r, note) in &loaded {
        for link in note.links() {
            total_links += 1;
            if !link.target.is_empty() {
                *unique_targets.entry(link.target.to_lowercase()).or_default() += 1;
            }
            match resolver.resolve(&link.target) {
                Resolution::Resolved | Resolution::SelfRef => {}
                Resolution::Ambiguous => ambiguous += 1,
                Resolution::Attachment => attachments += 1,
                Resolution::Broken => {
                    broken += 1;
                    broken_list.push((r.relative.clone(), link.target.clone()));
                }
            }
        }
        for tag in note.tags() {
            *tags.entry(tag).or_default() += 1;
        }
    }

    println!("vault:      {}", display_path(vault.root()));
    println!("notes:      {}", loaded.len());
    println!("links:      {total_links} ({} unique targets)", unique_targets.len());
    println!("broken:     {broken}");
    println!("ambiguous:  {ambiguous}");
    println!("attachments:{attachments}");
    println!("tags:       {}", tags.len());
    if read_errors > 0 {
        println!("unreadable: {read_errors}");
    }

    if command == "broken" {
        if broken_list.is_empty() {
            println!("\nno broken links");
        } else {
            println!("\nbroken links:");
            for (from, to) in &broken_list {
                println!("  {from}  ->  [[{to}]]");
            }
        }
    }

    Ok(())
}

/// Strip the Windows verbatim prefix (`\\?\`) that `canonicalize` adds.
fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s)
}

fn print_help() {
    println!(
        "dragon-glass — headless vault tooling\n\n\
         USAGE:\n  \
           dragon-glass stats  [vault]   summarize a vault\n  \
           dragon-glass broken [vault]   list unresolved [[links]]\n\n\
         vault defaults to the current directory."
    );
}

enum Resolution {
    Resolved,
    /// `[[#heading]]` — a link within the same file.
    SelfRef,
    Ambiguous,
    /// Target names a non-markdown file (image, pdf, …).
    Attachment,
    Broken,
}

struct Resolver<'a> {
    by_path: HashMap<String, &'a NoteRef>,
    by_stem: HashMap<String, Vec<&'a NoteRef>>,
}

impl<'a> Resolver<'a> {
    fn new(refs: impl Iterator<Item = &'a NoteRef>) -> Self {
        let mut by_path = HashMap::new();
        let mut by_stem: HashMap<String, Vec<&NoteRef>> = HashMap::new();
        for r in refs {
            by_path.insert(r.path_no_ext().to_lowercase(), r);
            by_stem.entry(r.stem().to_lowercase()).or_default().push(r);
        }
        Self { by_path, by_stem }
    }

    fn resolve(&self, target: &str) -> Resolution {
        if target.is_empty() {
            return Resolution::SelfRef;
        }

        let key = target.replace('\\', "/").to_lowercase();

        if self.by_path.contains_key(&key) {
            return Resolution::Resolved;
        }

        if let Some(ext) = Path::new(&key).extension().and_then(|e| e.to_str()) {
            if ext != "md" && ext != "markdown" {
                return Resolution::Attachment;
            }
        }

        match self.by_stem.get(&key) {
            Some(v) if v.len() == 1 => Resolution::Resolved,
            Some(_) => Resolution::Ambiguous,
            None => {
                let stem = key.rsplit('/').next().unwrap_or(&key);
                match self.by_stem.get(stem) {
                    Some(v) if !v.is_empty() => Resolution::Resolved,
                    _ => Resolution::Broken,
                }
            }
        }
    }
}
