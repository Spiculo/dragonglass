# Summary

<!-- What changed, and why? Link the issue this closes: "Closes #123". -->

## How it was tested

<!--
OS, command, and what you exercised. Screenshots or a short recording for visible UI
changes. Mention the vault size if performance is relevant.
-->

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features`
- [ ] `cargo test`
- [ ] `cargo build -p dragonglass-ui` (if the UI changed)
- [ ] The change keeps `dragonglass-core` free of UI dependencies (if it touches core)
- [ ] Notes are still plain markdown; no data-loss path was introduced
- [ ] No new network access, polling, or telemetry
- [ ] Documentation (README, PLAN, docs/) updated if behaviour or decisions changed
