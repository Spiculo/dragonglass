# Security Policy

## Supported versions

Dragon Glass is pre-1.0 and moves quickly. Security fixes land on `main` and in the newest
release; older point releases are not maintained separately.

| Version | Supported |
|---|---|
| latest release | ✅ |
| older releases | ❌ |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Use GitHub's private reporting: open the repository's **Security** tab and choose
**Report a vulnerability**. That creates a private advisory visible only to the maintainers.
If that is unavailable, contact [@FelipeLecot](https://github.com/FelipeLecot) privately
through the contact details on their GitHub profile.

Please include:

- the affected version or commit,
- operating system,
- a description of the issue and its impact,
- steps to reproduce, or a proof of concept,
- any suggested fix, if you have one.

You can expect an acknowledgement within a few days. Once a fix is ready, we will publish a
GitHub Security Advisory and credit you unless you prefer otherwise.

## Scope and threat model

Dragon Glass is designed to be offline-first. Useful context when assessing a report:

- **No network by default.** The app does not phone home, collect telemetry, or check for
  updates unless an update source is explicitly configured.
- **Git sync** shells out to the user's own `git` binary; credentials and SSH configuration
  stay with git, not with Dragon Glass.
- **Local HTTP API** (`dragon-glass serve`) binds to loopback and is read-only. Treat it as
  a local convenience, not a hardened server.
- **MCP server** (`dragon-glass mcp`) speaks over stdio and is read-only.
- **Plugins** run in a `wasmtime` sandbox and are experimental; only install plugins you
  trust, and review their manifests.
- **File handling** stays inside the vault. Path traversal, symlink escapes, and malicious
  markdown that causes arbitrary file reads or writes are in scope.
- **Exports** write to a directory the user chooses; rendering markdown/HTML must not
  execute scripts.

Vulnerabilities in third-party dependencies should be reported upstream as well; we still
want to know so we can bump the dependency.

## Dependency updates

Dependabot keeps Cargo and GitHub Actions dependencies current. A few dependencies are
pinned exactly on purpose (see the comment in `crates/dragonglass-core/Cargo.toml`); do not
"fix" those pins without checking the upstream issue they work around.
