//! `dragon-glass` — headless CLI over a vault.
//!
//! Usage:
//!   dragon-glass stats       [vault]
//!   dragon-glass broken      [vault]
//!   dragon-glass git-init    [vault]
//!   dragon-glass git-status  [vault]
//!   dragon-glass git-commit  [vault] <message>
//!   dragon-glass git-pull    [vault]
//!   dragon-glass git-push    [vault]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use dragonglass_core::{
    api, git, graph, history, html, query, storage, update, Note, NoteRef, Vault,
};
use dragonglass_plugins as plugins;

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
    let first = args.next();

    match first.as_deref() {
        None | Some("-h") | Some("--help") => {
            print_help();
            return Ok(());
        }
        Some("git-init") => return git_init(args.next()),
        Some("git-status") => return git_status(args.next()),
        Some("git-commit") => return git_commit(args.collect()),
        Some("git-pull") => return git_pull(args.next()),
        Some("git-push") => return git_push(args.next()),
        Some("history") => return history_cmd(args.collect()),
        Some("export") => return export(args.collect()),
        Some("query") => return query_cmd(args.collect()),
        Some("serve") => return serve(args.collect()),
        Some("check-update") => return check_update(args.collect()),
        Some("graph") => return graph_cmd(args.collect()),
        Some("plugins") => return plugins_cmd(args.collect()),
        Some("run") => return run_plugin(args.collect()),
        _ => {}
    }

    let (command, path) = match first.as_deref() {
        Some("stats") => ("stats", args.next()),
        Some("broken") => ("broken", args.next()),
        Some(other) => ("stats", Some(other.to_string())),
        None => unreachable!("handled above"),
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
                *unique_targets
                    .entry(link.target.to_lowercase())
                    .or_default() += 1;
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
    println!(
        "links:      {total_links} ({} unique targets)",
        unique_targets.len()
    );
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

fn vault_arg(path: Option<String>) -> PathBuf {
    PathBuf::from(path.unwrap_or_else(|| ".".to_string()))
}

fn git_init(path: Option<String>) -> Result<()> {
    let root = vault_arg(path);
    git::init(&root)?;
    println!("initialized git repository at {}", display_path(&root));
    Ok(())
}

fn git_status(path: Option<String>) -> Result<()> {
    let root = vault_arg(path);
    let status = git::status(&root)?;
    println!("branch:     {}", status.branch);
    println!("changed:    {}", status.changed);
    println!("untracked:  {}", status.untracked);
    println!("ahead:      {}", status.ahead);
    println!("behind:     {}", status.behind);
    if !status.conflicted.is_empty() {
        println!("conflicted: {}", status.conflicted.join(", "));
    }
    Ok(())
}

fn git_commit(args: Vec<String>) -> Result<()> {
    let (root, message) = match args.len() {
        0 => bail!("usage: dragon-glass git-commit [vault] <message>"),
        1 => (vault_arg(None), args[0].clone()),
        _ => (vault_arg(Some(args[0].clone())), args[1..].join(" ")),
    };

    if git::commit_all(&root, &message)? {
        println!("committed: {message}");
    } else {
        println!("nothing to commit");
    }
    Ok(())
}

fn git_pull(path: Option<String>) -> Result<()> {
    report_outcome(git::pull(&vault_arg(path))?);
    Ok(())
}

fn git_push(path: Option<String>) -> Result<()> {
    report_outcome(git::push(&vault_arg(path))?);
    Ok(())
}

fn plugins_cmd(args: Vec<String>) -> Result<()> {
    let root = vault_arg(args.first().cloned());
    let found = plugins::discover(&root);

    if found.is_empty() {
        println!(
            "no plugins installed in {}",
            display_path(&plugins::plugins_dir(&root))
        );
        return Ok(());
    }

    for plugin in found {
        let version = plugin.manifest.version.clone().unwrap_or_default();
        println!(
            "{}  {}  {}",
            plugin.manifest.id, plugin.manifest.name, version
        );
        for command in &plugin.manifest.commands {
            println!("    {}  {}", command.id, command.title);
        }
    }
    Ok(())
}

fn run_plugin(args: Vec<String>) -> Result<()> {
    if args.len() < 3 {
        bail!("usage: dragon-glass run <vault> <plugin-id> <command-id> [note]");
    }

    let root = PathBuf::from(&args[0]);
    let plugin_id = &args[1];
    let command = &args[2];
    let note = args.get(3).map(String::as_str);

    let found = plugins::discover(&root);
    let plugin = found
        .iter()
        .find(|plugin| plugin.manifest.id == *plugin_id)
        .ok_or_else(|| anyhow!("plugin not found: {plugin_id}"))?;

    println!("{}", plugins::run_command(plugin, command, &root, note)?);
    Ok(())
}

fn graph_cmd(args: Vec<String>) -> Result<()> {
    let root = PathBuf::from(args.first().cloned().unwrap_or_else(|| ".".to_string()));
    let vault =
        Vault::open(&root).with_context(|| format!("opening vault {}", display_path(&root)))?;

    let full = graph::build(&vault.notes());

    let view = match args.get(1) {
        Some(note) => match full.index_of(note) {
            Some(id) => graph::local(&full, id, 2, 100),
            None => {
                println!("not in graph: {note}");
                return Ok(());
            }
        },
        None => full,
    };

    println!("nodes: {}  edges: {}", view.nodes.len(), view.edges.len());

    let mut nodes = view.nodes.clone();
    nodes.sort_by(|a, b| {
        b.links
            .cmp(&a.links)
            .then_with(|| a.relative.cmp(&b.relative))
    });
    for node in nodes.iter().take(25) {
        println!("  {:<44} links={}", node.relative, node.links);
    }
    Ok(())
}

fn check_update(args: Vec<String>) -> Result<()> {
    let Some(source) = args.first() else {
        bail!("usage: dragon-glass check-update <source> [current-version]");
    };

    let current = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    match update::latest_if_newer(&current, source)? {
        Some(latest) => println!("update available: {latest} (running {current})"),
        None => println!("up to date ({current})"),
    }
    Ok(())
}

fn serve(args: Vec<String>) -> Result<()> {
    let root = PathBuf::from(args.first().cloned().unwrap_or_else(|| ".".to_string()));
    let addr = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "127.0.0.1:7456".to_string());

    println!("serving {} at http://{addr}", display_path(&root));
    api::serve(&root, &addr).map_err(Into::into)
}

fn query_cmd(args: Vec<String>) -> Result<()> {
    if args.len() < 2 {
        bail!("usage: dragon-glass query <vault> \"<query>\"");
    }

    let root = PathBuf::from(&args[0]);
    let expression = args[1..].join(" ");
    let vault =
        Vault::open(&root).with_context(|| format!("opening vault {}", display_path(&root)))?;
    let rows = query::run(&vault.notes(), &expression);

    if rows.is_empty() {
        println!("no matches");
        return Ok(());
    }

    println!("{:<40} properties", "note");
    for row in rows {
        let properties = row
            .values
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("  ");
        println!("{:<40} {}", row.relative, properties);
    }
    Ok(())
}

fn export(args: Vec<String>) -> Result<()> {
    if args.len() < 2 {
        bail!("usage: dragon-glass export <vault> <out-dir>");
    }

    let root = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    let vault =
        Vault::open(&root).with_context(|| format!("opening vault {}", display_path(&root)))?;

    std::fs::create_dir_all(&out)?;

    let notes = vault.notes();
    let mut index = String::from("<h1>Index</h1>\n<ul>\n");
    let mut exported = 0usize;

    for note_ref in &notes {
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };

        let file_name = format!("{}.html", note_ref.path_no_ext().replace('/', "__"));
        let document = html::document(note_ref.stem(), &html::render_body(&note.source));
        storage::write_atomic(&out.join(&file_name), &document)?;

        index.push_str(&format!(
            "<li><a href=\"{}\">{}</a></li>\n",
            html::escape(&file_name),
            html::escape(note_ref.stem())
        ));
        exported += 1;
    }

    index.push_str("</ul>\n");
    storage::write_atomic(&out.join("index.html"), &html::document("Index", &index))?;

    println!("exported {exported} notes to {}", display_path(&out));
    Ok(())
}

fn history_cmd(args: Vec<String>) -> Result<()> {
    if args.len() < 2 {
        bail!("usage: dragon-glass history <vault> <note-relative-path> [snapshot-id]");
    }

    let root = PathBuf::from(&args[0]);
    let note = args[1].clone();

    if let Some(id) = args.get(2) {
        print!("{}", history::read(&root, &note, id)?);
        return Ok(());
    }

    let ids = history::list(&root, &note);
    if ids.is_empty() {
        println!("no snapshots for {note}");
        return Ok(());
    }
    for id in ids {
        println!("{id}");
    }
    Ok(())
}

fn report_outcome(outcome: git::Outcome) {
    match outcome {
        git::Outcome::UpToDate => println!("already up to date"),
        git::Outcome::Updated => println!("updated"),
        git::Outcome::NoRemote => println!("no remote configured"),
        git::Outcome::Conflicts(files) => println!("{}", git::conflict_instructions(&files)),
    }
}

fn print_help() {
    println!(
        "dragon-glass — headless vault tooling\n\n\
         USAGE:\n  \
           dragon-glass stats       [vault]           summarize a vault\n  \
           dragon-glass broken      [vault]           list unresolved [[links]]\n  \
           dragon-glass git-init    [vault]           create a git repository\n  \
           dragon-glass git-status  [vault]           show git status\n  \
           dragon-glass git-commit  [vault] <message> stage everything and commit\n  \
           dragon-glass git-pull    [vault]           fast-forward from the remote\n  \
           dragon-glass git-push    [vault]           push to the remote\n  \
           dragon-glass history     <vault> <note>    list snapshots for a note\n  \
           dragon-glass history     <vault> <note> <id>  print a snapshot\n  \
           dragon-glass export      <vault> <out-dir>   render notes to HTML\n  \
           dragon-glass query       <vault> \"<query>\"  filter notes by property or tag\n  \
           dragon-glass serve       [vault] [addr]     read-only HTTP API (loopback)\n  \
           dragon-glass check-update <source> [current]  compare against an advertised version\n  \
           dragon-glass graph       [vault] [note]     print the link graph\n  \
           dragon-glass plugins     [vault]            list installed plugins\n  \
           dragon-glass run         <vault> <plugin> <command> [note]  run a plugin command\n\n\
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
