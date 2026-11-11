# Dragon Glass for Android

A native Android shell around `dragonglass-core`. The Rust engine is exposed through
`crates/dragonglass-ffi` (UniFFI), and the UI is Kotlin + Jetpack Compose.

This is the first vertical slice, not full parity yet:

- Open the app-private vault and list notes, either flat or as a folder tree
- Fuzzy title search plus full-text search with line-level hits that jump into the editor
- Bookmarks, stored in the vault's `.dragon-glass` folder, with a bookmarks-only filter
- One-commit git sync with a GitHub HTTPS token
- Edit markdown source and save atomically
- Render markdown/HTML (headings, lists, tables, math, code) in a WebView
- Outline, properties, stats, relations and tags from the shared engine
- Daily, weekly and monthly notes from `Templates/`
- A graph view of links, embeds and frontmatter relations
- Create notes from the vault's markdown templates
- Rename or move notes, create folders, and delete through a recoverable trash that can be
  browsed, restored or emptied
- Import a vault folder from device storage, keep it linked, and write saves back through
  the system folder picker (Android SAF) while keeping several vaults side by side

Still to come: true in-place editing of a `content://` vault (the app edits a local mirror
and syncs through SAF), themes, workspaces, plugin support, and git-aware features that do
not make sense on Android.

## Prerequisites

- Android Studio (SDK 37, build tools installed)
- Rust with Android targets:

  ```powershell
  rustup target add aarch64-linux-android x86_64-linux-android
  ```

- `cargo-ndk`:

  ```powershell
  cargo install cargo-ndk --locked
  ```

- Android NDK. In Android Studio: **SDK Manager → SDK Tools → NDK (Side by side)**.
  Or from the command line:

  ```powershell
  sdkmanager "ndk;27.2.12479018"
  ```

## Build the native library

From the repository root:

```powershell
.\mobile\android\scripts\build-native.ps1
```

On macOS/Linux:

```bash
./mobile/android/scripts/build-native.sh
```

The script builds `crates/dragonglass-ffi`, regenerates the Kotlin bindings in
`app/src/main/java/uniffi/dragonglass/`, and cross-compiles `libdragonglass_ffi.so` for
`arm64-v8a` and `x86_64` into `app/src/main/jniLibs/`.

## Build and run the app

```powershell
cd mobile\android
.\gradlew.bat :app:assembleDebug
```

Or open `mobile/android` in Android Studio and run the `app` configuration.

## Git sync

The app uses JGit for a native Android git client. Desktop keeps using the system `git`
binary; both read and write the same repository format.

1. Create a **fine-grained personal access token** on GitHub with
   **Contents: Read and write** for the vault repository.
2. Tap **Sync** in the note list.
3. Enter `https://github.com/<owner>/<repo>.git`, the branch, and the token.
4. Use **Link existing** to clone the GitHub repo into the vault (the old local vault is
   kept as a timestamped backup), or **Create new** to initialize a new repository and push
   the current notes. **Create new** expects an empty remote.
5. After that, **Sync** does `commit all changes as one commit → pull --rebase → push`.

The token is encrypted with Android Keystore and never written into the vault or
`.git/config`. Local `file://` remotes are supported for testing and do not require a token.

## Storage model

The engine uses `std::fs`, so every vault lives in app-private storage:

- the default vault is `filesDir/vault`
- imported vaults are copied into `filesDir/vaults/<id>` and stay linked to the SAF folder
  they came from

**Import folder…** (in the vaults sheet, opened from the vault name in the top bar) copies a
folder tree from the system picker into a new app-private vault. While a vault is linked:

- saving a note writes that note back to the folder
- creating a folder, renaming or moving, moving to the trash, and restoring mirror the whole
  vault and prune files that no longer exist locally
- opening the vault pulls files whose folder timestamp is newer than the local copy
- **Sync with folder** pulls newer files and mirrors everything in one step

The app still edits the app-private copy, not `content://` documents directly, so the mirror
stays the source of truth. Remote deletions are not propagated (a locally present note is
written back); edits made elsewhere are only picked up by the timestamp comparison on open or
by **Sync with folder**. True in-place vault access needs a storage abstraction (SAF
`content://` trees on Android, security-scoped bookmarks on iOS).
