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
    api, assets, git, graph, history, html, import, mcp, pdf, query, resolve, shared, storage,
    store, update, uri, Note, NoteRef, Vault,
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
        Some("uri") => return uri_cmd(args.collect()),
        Some("import") => return import_cmd(args.collect()),
        Some("mcp") => return mcp_cmd(args.collect()),
        Some("store") => return store_cmd(args.collect()),
        Some("install") => return install_cmd(args.collect()),
        Some("pdf") => return pdf_cmd(args.collect()),
        Some("register-uri") => return register_uri(args.collect(), false),
        Some("unregister-uri") => return register_uri(args.collect(), true),
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

    let index = resolve::NoteIndex::new(
        &loaded
            .iter()
            .map(|(note, _)| note.clone())
            .collect::<Vec<_>>(),
    );

    let mut total_links = 0usize;
    let mut broken = 0usize;
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
            match classify(&index, &link.target) {
                Resolution::Resolved | Resolution::SelfRef => {}
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

/// One filesystem or process change needed to register the URI scheme.
///
/// `WriteFile`/`RemoveFile` are only used on Linux, hence the dead-code allowance.
#[allow(dead_code)]
enum Step {
    Run(Vec<String>),
    WriteFile(PathBuf, String),
    RemoveFile(PathBuf),
}

impl Step {
    fn describe(&self) -> String {
        match self {
            Step::Run(parts) => parts.join(" "),
            Step::WriteFile(path, _) => format!("write {}", display_path(path)),
            Step::RemoveFile(path) => format!("delete {}", display_path(path)),
        }
    }

    fn apply(&self) -> Result<()> {
        match self {
            Step::Run(parts) => {
                let (program, arguments) = parts
                    .split_first()
                    .ok_or_else(|| anyhow!("empty command"))?;
                let status = std::process::Command::new(program)
                    .args(arguments)
                    .status()?;
                if !status.success() {
                    bail!("command failed: {}", parts.join(" "));
                }
            }
            Step::WriteFile(path, contents) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(path, contents)?;
            }
            Step::RemoveFile(path) => {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }
}

/// Register (or unregister) the `dragonglass://` handler. Dry run unless `--apply`.
fn register_uri(args: Vec<String>, unregister: bool) -> Result<()> {
    let apply = args.iter().any(|argument| argument == "--apply");
    let explicit = args
        .iter()
        .find(|argument| !argument.starts_with("--"))
        .cloned();

    let exe = match explicit {
        Some(path) => path,
        None => std::env::current_exe()
            .map(|path| path.to_string_lossy().to_string())
            .map_err(|_| anyhow!("cannot determine the executable path; pass it as an argument"))?,
    };

    let steps = if unregister {
        unregister_plan()
    } else {
        register_plan(&exe)
    };

    if steps.is_empty() {
        println!("URI registration is not implemented on this platform");
        return Ok(());
    }

    for step in &steps {
        println!("{}", step.describe());
    }

    if apply {
        for step in &steps {
            step.apply()?;
        }
        println!("applied {} step(s)", steps.len());
    } else {
        println!("(dry run; pass --apply to make these changes)");
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn register_plan(exe: &str) -> Vec<Step> {
    let key = r"HKCU\Software\Classes\dragonglass";
    vec![
        Step::Run(vec![
            "reg".into(),
            "add".into(),
            key.into(),
            "/ve".into(),
            "/d".into(),
            "URL:Dragon Glass protocol".into(),
            "/f".into(),
        ]),
        Step::Run(vec![
            "reg".into(),
            "add".into(),
            key.into(),
            "/v".into(),
            "URL Protocol".into(),
            "/d".into(),
            String::new(),
            "/f".into(),
        ]),
        Step::Run(vec![
            "reg".into(),
            "add".into(),
            format!("{key}\\shell\\open\\command"),
            "/ve".into(),
            "/d".into(),
            format!("\"{exe}\" \"%1\""),
            "/f".into(),
        ]),
    ]
}

#[cfg(target_os = "windows")]
fn unregister_plan() -> Vec<Step> {
    vec![Step::Run(vec![
        "reg".into(),
        "delete".into(),
        r"HKCU\Software\Classes\dragonglass".into(),
        "/f".into(),
    ])]
}

#[cfg(target_os = "linux")]
fn desktop_file_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/share/applications/dragon-glass.desktop")
}

#[cfg(target_os = "linux")]
fn register_plan(exe: &str) -> Vec<Step> {
    let contents = format!(
        "[Desktop Entry]\nType=Application\nName=Dragon Glass\nExec={exe} %u\n\
         Terminal=false\nMimeType=x-scheme-handler/dragonglass;\nNoDisplay=true\n"
    );

    vec![
        Step::WriteFile(desktop_file_path(), contents),
        Step::Run(vec![
            "xdg-mime".into(),
            "default".into(),
            "dragon-glass.desktop".into(),
            "x-scheme-handler/dragonglass".into(),
        ]),
    ]
}

#[cfg(target_os = "linux")]
fn unregister_plan() -> Vec<Step> {
    vec![Step::RemoveFile(desktop_file_path())]
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn register_plan(_exe: &str) -> Vec<Step> {
    Vec::new()
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn unregister_plan() -> Vec<Step> {
    Vec::new()
}

fn pdf_cmd(args: Vec<String>) -> Result<()> {
    if args.len() < 3 {
        bail!("usage: dragon-glass pdf <vault> <note-relative-path> <out.pdf>");
    }

    let root = PathBuf::from(&args[0]);
    let note = &args[1];
    let out = PathBuf::from(&args[2]);

    let source =
        std::fs::read_to_string(root.join(note)).with_context(|| format!("reading note {note}"))?;
    let title = Path::new(note)
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default();

    let source = shared::expand(&source, &root);
    let document = pdf::from_markdown(&source, &title);
    storage::write_atomic(&out, &document)?;

    println!("wrote {} ({} bytes)", display_path(&out), document.len());
    Ok(())
}

fn store_cmd(args: Vec<String>) -> Result<()> {
    let Some(directory) = args.first() else {
        bail!("usage: dragon-glass store <store-dir> [query]");
    };

    let store_path = PathBuf::from(directory);
    let query = args.get(1).map(|value| value.to_ascii_lowercase());

    let packages: Vec<store::Package> = store::scan(&store_path)
        .into_iter()
        .filter(|package| {
            query
                .as_ref()
                .map(|query| {
                    package.id.to_lowercase().contains(query)
                        || package.name.to_lowercase().contains(query)
                })
                .unwrap_or(true)
        })
        .collect();

    if packages.is_empty() {
        println!("no packages in {}", display_path(&store_path));
        return Ok(());
    }

    for package in packages {
        let kind = match package.kind {
            store::PackageKind::Plugin => "plugin",
            store::PackageKind::Theme => "theme",
        };
        println!(
            "{:<20} {:<7} {:<24} {}",
            package.id,
            kind,
            package.name,
            package.version.unwrap_or_default()
        );
    }
    Ok(())
}

fn install_cmd(args: Vec<String>) -> Result<()> {
    if args.len() < 3 {
        bail!("usage: dragon-glass install <vault> <store-dir> <package-id>");
    }

    let vault = PathBuf::from(&args[0]);
    let store_path = PathBuf::from(&args[1]);
    let id = &args[2];

    let packages = store::scan(&store_path);
    let package = packages
        .iter()
        .find(|package| package.id == *id)
        .ok_or_else(|| anyhow!("package not found: {id}"))?;

    let installed = store::install(&vault, package)?;
    println!("installed {id} to {}", display_path(&installed));
    Ok(())
}

fn mcp_cmd(args: Vec<String>) -> Result<()> {
    let root = vault_arg(args.first().cloned());
    mcp::serve(&root).map_err(Into::into)
}

fn import_cmd(args: Vec<String>) -> Result<()> {
    if args.len() < 3 {
        bail!("usage: dragon-glass import <vault> <enex|roam> <file> [folder]");
    }

    let root = PathBuf::from(&args[0]);
    let format = args[1].to_ascii_lowercase();
    let source = PathBuf::from(&args[2]);
    let folder = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "Imported".to_string());

    let notes = match format.as_str() {
        "enex" | "evernote" => {
            let text = std::fs::read_to_string(&source)
                .with_context(|| format!("reading {}", display_path(&source)))?;
            import::evernote_enex(&text)
        }
        "roam" => {
            let text = std::fs::read_to_string(&source)
                .with_context(|| format!("reading {}", display_path(&source)))?;
            import::roam_pages(&text)?
        }
        // A Notion export is a directory of markdown files.
        "notion" => import::notion_export(&source),
        other => bail!("unknown import format: {other} (expected enex, roam or notion)"),
    };

    if notes.is_empty() {
        println!("no notes found in {}", display_path(&source));
        return Ok(());
    }

    let out_dir = root.join(&folder);
    std::fs::create_dir_all(&out_dir)?;

    let mut written = 0usize;
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();

    for note in &notes {
        let base = sanitize_file_name(&note.title);
        let base = if base.is_empty() {
            "Untitled".to_string()
        } else {
            base
        };

        let mut name = base.clone();
        let mut counter = 1;
        while !used.insert(name.clone()) {
            counter += 1;
            name = format!("{base} {counter}");
        }

        storage::write_atomic(&out_dir.join(format!("{name}.md")), &note.markdown)?;
        written += 1;
    }

    println!("imported {written} notes into {}", display_path(&out_dir));
    Ok(())
}

fn sanitize_file_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn uri_cmd(args: Vec<String>) -> Result<()> {
    let Some(raw) = args.first() else {
        bail!("usage: dragon-glass uri <dragonglass://...>");
    };

    let parsed = uri::parse(raw).ok_or_else(|| anyhow!("not a dragonglass URI: {raw}"))?;
    println!("action: {}", parsed.action);
    if let Some(vault) = &parsed.vault {
        println!("vault:  {vault}");
    }
    if let Some(note) = &parsed.note {
        let path = parsed
            .vault
            .as_ref()
            .map(|vault| PathBuf::from(vault).join(note))
            .unwrap_or_else(|| PathBuf::from(note));
        println!(
            "note:   {} (exists: {})",
            display_path(&path),
            path.exists()
        );
    }
    if let Some(query) = &parsed.query {
        println!("query:  {query}");
    }
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
    let attachments = assets::Attachments::index(&root);
    let mut used: Vec<String> = Vec::new();
    let mut index = String::from("<h1>Index</h1>\n<ul>\n");
    let mut exported = 0usize;

    for note_ref in &notes {
        let Ok(note) = Note::load(&note_ref.path) else {
            continue;
        };

        let file_name = format!("{}.html", note_ref.path_no_ext().replace('/', "__"));
        let source = shared::expand(&note.source, &root);
        let body = html::render_body_with(&source, Some(&attachments), &mut used);
        let document = html::document(note_ref.stem(), &body);
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

    // Attachments keep their vault-relative paths, so the `src` written into each page
    // resolves without rewriting.
    let mut copied = 0usize;
    for relative in &used {
        let from = root.join(relative);
        let to = out.join(relative);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::copy(&from, &to).is_ok() {
            copied += 1;
        }
    }

    println!(
        "exported {exported} notes and {copied} attachments to {}",
        display_path(&out)
    );
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
           dragon-glass run         <vault> <plugin> <command> [note]  run a plugin command\n  \
           dragon-glass uri         <dragonglass://...>  resolve a URI\n  \
           dragon-glass import      <vault> <enex|roam|notion> <file|dir> [folder]  import notes\n  \
           dragon-glass mcp         [vault]            MCP server over stdio (read-only)\n  \
           dragon-glass store       <store-dir> [query]  list installable packages\n  \
           dragon-glass install     <vault> <store-dir> <id>  install a package\n  \
           dragon-glass pdf         <vault> <note> <out.pdf>  print a note to PDF\n  \
           dragon-glass register-uri [exe] [--apply]   register the dragonglass scheme\n  \
           dragon-glass unregister-uri [--apply]       remove the scheme registration\n\n\
         vault defaults to the current directory."
    );
}

enum Resolution {
    Resolved,
    /// `[[#heading]]` — a link within the same file.
    SelfRef,
    /// Target names a non-markdown file (image, pdf, …).
    Attachment,
    Broken,
}

/// Classify a link target against the vault index (paths, stems, slugs and aliases).
fn classify(index: &resolve::NoteIndex, target: &str) -> Resolution {
    if target.is_empty() {
        return Resolution::SelfRef;
    }

    if index.resolve(target).is_some() {
        return Resolution::Resolved;
    }

    if let Some(ext) = Path::new(target).extension().and_then(|e| e.to_str()) {
        if !ext.eq_ignore_ascii_case("md") && !ext.eq_ignore_ascii_case("markdown") {
            return Resolution::Attachment;
        }
    }

    Resolution::Broken
}
