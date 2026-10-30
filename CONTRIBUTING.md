# Contributing to Dragon Glass

Thanks for taking the time to help. Dragon Glass is a native, local-first markdown editor;
the bar for changes is "does this keep the app fast, safe with the user's notes, and native?"

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Before you start

- Read [`PLAN.md`](PLAN.md). It records the scope, roadmap and a list of **locked design
  decisions** (GPUI, GPL, no webview, Obsidian keybindings, plain-text config, and so on).
  A pull request that reopens a settled decision is likely to be redirected there first.
- For anything larger than a small fix, open an issue and describe the problem before
  writing code. It avoids duplicated work and saves a painful review.
- Small, focused pull requests are much easier to review than one large one.

## Set up

You need Rust 1.85+ (stable) and `git`. Linux system packages are listed in the
[README](README.md#build-from-source). Then:

```bash
git clone https://github.com/Spiculo/dragonglass.git
cd dragonglass
cargo run -p dragonglass-ui -- /path/to/a/vault
```

Use a copy of a real vault, or an empty folder, while developing. The repository ignores
`test-vault/` on purpose: never commit a personal vault or note contents.

## Checks before opening a pull request

CI runs these on Windows and Linux; run them locally first:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features
cargo test
cargo build -p dragonglass-ui
```

The first three use the workspace's default members (core, CLI and plugins). The UI is
built separately because GPUI is heavy.

If you change parser, editor or layout behaviour, add or update a test in
`dragonglass-core` when the logic can live there. The core crate is deliberately headless
and unit-testable — that is why it exists.

## Code guidelines

- **Keep `dragonglass-core` free of UI dependencies.** It is a library shared by the app,
  the CLI, tests and (later) other front ends.
- **Data safety first.** Never overwrite a note non-atomically, and never discard unsaved
  edits silently. Reuse `storage::write_atomic` and the conflict handling in the UI.
- **No network by default.** New network features need an explicit opt-in and a design
  discussion. Telemetry is out of scope.
- **Stay event-driven.** Do not add polling loops or timers to catch file changes; the idle
  CPU budget is a headline feature. Use the existing `notify` watcher.
- **Match Obsidian's default keybindings** where a matching command exists.
- **Windows and Linux first.** Use `Path`/`PathBuf` (never string-concatenate paths), and
  keep path comparisons case-insensitive on Windows (see `vaults::canonical`).
- **Small, hand-rolled formats for config.** `.dragon-glass/*.txt` files are `key=value`
  or one item per line on purpose: no dependency, diffable, obvious to edit.
- Follow the surrounding style; `cargo fmt` is the source of truth for layout.

## Commits and pull requests

- Write imperative, descriptive commit subjects: `Fix caret after wrapped lines`, not
  `fixed stuff`.
- One logical change per commit when practical.
- In the pull request, explain **what** changed and **why**, and note how you tested it
  (OS, command, vault size). Link the issue it closes.
- Include screenshots or a short recording for visible UI changes.
- Keep the PR template checklist honest; it is fine to leave an item unticked with a note.

## Reporting bugs

Use the bug report template and include:

- the Dragon Glass version or commit,
- operating system and whether it is a release or debug build,
- exact steps to reproduce,
- what you expected and what happened,
- relevant console output.

**Do not paste private note contents.** A minimal synthetic `.md` file that reproduces the
problem is almost always enough. Security issues go through [`SECURITY.md`](SECURITY.md).

## Good first issues

Areas that are friendly to new contributors:

- parser and reading-view edge cases (fed by `dragonglass-core` tests),
- CLI output and importer gaps,
- documentation,
- Windows/Linux file-dialog and path-handling polish.

If you are unsure where to start, open an issue and ask.
