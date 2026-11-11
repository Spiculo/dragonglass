# iOS port: feasibility and plan

**Status:** Assessment only — no iOS code exists yet.
**Date:** 2026-11-11
**Scope:** Reaching feature parity with the current Android vertical slice
(`mobile/android`), not full desktop parity.

## Summary

The engine ports almost for free. The work is a SwiftUI client and a git client, not Rust.

- `dragonglass-core` is pure Rust with no C toolchain, system libraries or process
  spawning in any code path the mobile bindings use, so it should cross-compile to
  `aarch64-apple-ios` as-is.
- `crates/dragonglass-ffi` already builds a `staticlib` and UniFFI generates Swift as well
  as Kotlin from the same definitions (`src/lib.rs:5` explicitly anticipates "Kotlin and
  Swift callers"). All exported functions — vault, search, graph, tags, templates, daily
  notes, `render_html` — come across without changes.
- Markdown/HTML rendering, math layout, PDF generation, stats and relations all live in
  core and port unchanged. Only the WebView wrapper changes (`WKWebView`).
- The app-private vault model maps directly: `filesDir/vault` becomes
  `Documents/vault` (or `Application Support/vault`).

`PLAN.md` already anticipated this: "iOS can reuse the same bridge with Swift bindings."

Estimated effort for one experienced developer: **4–6 weeks** to Android-slice parity,
of which roughly **1 week is Rust/tooling** and the rest is Swift, git sync and
packaging.

## What ports unchanged

| Area | Why it ports |
| --- | --- |
| Vault model, tree, note read/write | `dragonglass-core` uses only `std::fs` on plain paths |
| Search (fuzzy + full text) | pure Rust |
| Markdown → HTML, wikilinks, math | pure Rust (`ratex-*`, `ab_glyph`, embedded fonts) |
| PDF export | pure Rust, no external tool (`core/src/pdf.rs`) |
| Graph, relations, tags, stats | pure Rust |
| Bookmarks, templates, daily/weekly/monthly notes | pure Rust, written under `.dragon-glass/` |
| Atomic saves | `core/src/storage.rs`, `rename` over a temp file |

Desktop-only modules are irrelevant to the port:

- `core/src/git.rs` shells out to the user's `git` binary, and
- `core/src/update.rs` opens a raw TCP connection.

Neither is exposed through `crates/dragonglass-ffi`.

## What needs new work

| Piece | Effort | Notes |
| --- | --- | --- |
| Rust → iOS build pipeline | 2–4 days | Two targets (`aarch64-apple-ios`, `aarch64-apple-ios-sim`), `xcframework` assembly, modulemap, link `libdragonglass_ffi.a` in Xcode. Replaces `mobile/android/scripts/build-native.ps1`. |
| SwiftUI client | 2–3 weeks | Port of `Screens.kt` (~760 lines) + `DragonGlassViewModel.kt` (~540) + theme. No code sharing with Compose; this is a rewrite, though against a small, stable API. |
| Git sync | 1–2 weeks | iOS has no `git` binary. Either port `GitSyncManager.kt` to libgit2 (SwiftGit2/ObjectiveGit), or — preferred — add `git2` to the FFI behind a feature flag so both platforms share one implementation, and Android can eventually drop JGit. |
| Token storage | 1–2 days | `SecureTokenStore.kt` → Keychain, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`. |
| Signing, TestFlight, CI | 3–5 days | Extend the existing `macos-latest` CI jobs. |

## iOS-specific constraints

1. **No process spawning.** Anything that shells out is unavailable. That rules out the
   desktop `git` module and any future "run a CLI" feature.
2. **No JIT, no `dlopen` of downloaded code.** `dragonglass-plugins` runs `wasmtime` with
   the Cranelift JIT, which is not permitted on iOS. Plugin support there needs an
   interpreter backend (wasmtime's Pulley, `wasmi`, `wasm3`) and an App Store review
   argument under guideline 2.5.2. Plugins are not in the mobile slice today; this only
   caps future parity.
3. **Static linking.** The FFI must be linked as a static library into the app binary;
   there is no side-loading a `.dylib`.
4. **Filesystem access.** App-private storage is identical to Android. Reading
   user-chosen vaults needs `UIDocumentPickerViewController` plus security-scoped
   bookmarks. That gap already exists on Android (SAF), so iOS adds no new parity debt —
   but it is the single most-requested feature on both platforms.
5. **Licensing — the real blocker.** The project is GPL-3.0-or-later. GPLv3's
   anti-DRM/installation-information terms are generally considered incompatible with the
   Apple App Store terms of service. Shipping on the App Store likely requires either a
   GPL linking/distribution exception for iOS or distribution outside the App Store. This
   must be decided before Swift work starts; it is a legal question, not a technical one.
6. **Binary artifacts stay out of the repo.** `mobile/android/app/src/main/jniLibs/` is
   in `.gitignore` (the build script produces it), so iOS `.xcframework` output should be
   ignored the same way.

## Recommended sequence

1. **De-risk the port (1 day).** Build `dragonglass-ffi` for `aarch64-apple-ios`, generate
   Swift bindings, and run a 20-line XCTest that opens a vault and lists notes. This proves
   the core cross-compiles cleanly before any UI is written.
2. **Move git into the FFI.** Add `git2` to `crates/dragonglass-ffi` (feature-gated) so
   `commit → pull --rebase → push` has one implementation. Removes the need for a
   Swift-specific git library and gives Android a path off JGit.
3. **Thin SwiftUI shell.** Note list → editor → preview → search, then tags/outline/stats,
   then git UI. Mirror the Android screen structure so both stay conceptually aligned.
4. **Packaging.** Xcode project (or Tuist/XcodeGen), `.xcframework` build script, CI job on
   `macos-latest`, then signing/TestFlight once the licensing question is settled.

## Open questions

- [ ] Licensing: is an iOS/App Store distribution exception acceptable for a GPLv3 project?
- [ ] UI ownership: maintain two hand-written clients (Kotlin + Swift) indefinitely, or keep
      the mobile UI intentionally thin so ports stay cheap?
- [ ] Should git sync move into `dragonglass-core`/FFI (shared by Android, iOS and the CLI)
      rather than staying in platform UI code?

## Appendix: build sketch

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

cargo build --release -p dragonglass-ffi --target aarch64-apple-ios
cargo build --release -p dragonglass-ffi --target aarch64-apple-ios-sim

# Swift bindings from the host build of the library
cargo run -p dragonglass-ffi-bindgen -- generate \
  --library target/debug/libdragonglass_ffi.dylib \
  --language swift --out-dir mobile/ios/Bindings

xcodebuild -create-xcframework \
  -library target/aarch64-apple-ios/release/libdragonglass_ffi.a \
  -library target/aarch64-apple-ios-sim/release/libdragonglass_ffi.a \
  -output mobile/ios/DragonGlassFFI.xcframework
```

This mirrors `mobile/android/scripts/build-native.ps1`, which builds the host library,
generates Kotlin bindings, then cross-compiles with `cargo-ndk`.
