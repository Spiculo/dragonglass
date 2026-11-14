package com.dragonglass.app

import android.app.Application
import android.content.Context
import android.net.Uri
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.dragonglass.app.storage.StoredVault
import com.dragonglass.app.storage.VaultStore
import com.dragonglass.app.sync.GitStatus
import com.dragonglass.app.sync.GitSyncException
import com.dragonglass.app.sync.GitSyncManager
import com.dragonglass.app.sync.SecureTokenStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.dragonglass.DraftInfo
import uniffi.dragonglass.GraphView
import uniffi.dragonglass.HeadingInfo
import uniffi.dragonglass.MobileVault
import uniffi.dragonglass.NoteSummary
import uniffi.dragonglass.PropertyInfo
import uniffi.dragonglass.RelationInfo
import uniffi.dragonglass.SearchHit
import uniffi.dragonglass.StatsInfo
import uniffi.dragonglass.SyncStateInfo
import uniffi.dragonglass.syncState
import uniffi.dragonglass.TagCount
import uniffi.dragonglass.TemplateInfo
import uniffi.dragonglass.TrashEntry
import uniffi.dragonglass.TreeRow
import uniffi.dragonglass.documentStats
import uniffi.dragonglass.headings as parseHeadings
import uniffi.dragonglass.properties as parseProperties
import uniffi.dragonglass.findInText
import uniffi.dragonglass.renderHtmlFromVault
import java.io.File

private data class OpenNote(
    val relative: String,
    val source: String,
    val fingerprint: ULong?,
    val draft: DraftInfo?,
    val stats: StatsInfo,
    val headings: List<HeadingInfo>,
    val properties: List<PropertyInfo>,
    val relations: List<RelationInfo>,
    /// Rendered preview HTML when the link was followed from preview mode.
    val html: String = "",
)

/// How the note list is presented: a flat, recent-first list or the vault tree.
enum class BrowseMode { List, Tree }

class DragonGlassViewModel(application: Application) : AndroidViewModel(application) {
    private val vaultStore = VaultStore(application)
    private var activeVault = vaultStore.activeVault()
    private var vaultDirectory = activeVault.root
    private val gitPreferences =
        application.getSharedPreferences("dragon_glass_git", Context.MODE_PRIVATE)
    private val tokenStore = SecureTokenStore(application)
    private val uiPreferences =
        application.getSharedPreferences("dragon_glass_ui", Context.MODE_PRIVATE)
    private var vault: MobileVault? = null

    var vaults by mutableStateOf(vaultStore.vaults())
        private set
    var activeVaultId by mutableStateOf(activeVault.id)
        private set
    var canSyncWithFolder by mutableStateOf(activeVault.treeUri != null)
        private set
    var baseUrl by mutableStateOf("file://${vaultDirectory.absolutePath}/")
        private set
    var vaultLabel by mutableStateOf(activeVault.name)
        private set

    var notes by mutableStateOf<List<NoteSummary>>(emptyList())
        private set
    var treeRows by mutableStateOf<List<TreeRow>>(emptyList())
        private set
    var collapsedDirs by mutableStateOf<Set<String>>(emptySet())
        private set
    var browseMode by mutableStateOf(BrowseMode.List)
        private set
    var bookmarksOnly by mutableStateOf(false)
        private set
    var query by mutableStateOf("")
        private set
    var results by mutableStateOf<List<NoteSummary>>(emptyList())
        private set
    var hits by mutableStateOf<List<SearchHit>>(emptyList())
        private set
    var pendingLine by mutableStateOf<Int?>(null)
        private set
    var templates by mutableStateOf<List<TemplateInfo>>(emptyList())
        private set
    var graph by mutableStateOf<GraphView?>(null)
        private set
    var graphLoading by mutableStateOf(false)
        private set
    var trash by mutableStateOf<List<TrashEntry>>(emptyList())
        private set
    var trashLoading by mutableStateOf(false)
        private set
    var selected by mutableStateOf<String?>(null)
        private set
    var source by mutableStateOf("")
        private set
    var preview by mutableStateOf(false)
    /// WebView scroll offset, kept when switching between Edit and Preview.
    var previewScroll by mutableStateOf(0)
        private set

    /// Reading typography, shared with the desktop reader through the vault config.
    var readerSize by mutableStateOf(15)
        private set
    var readerMeasure by mutableStateOf(780)
        private set
    var readerFont by mutableStateOf("serif")
        private set

    fun setReaderTypography(size: Int, measure: Int, font: String = readerFont) {
        readerSize = size.coerceIn(11, 24)
        readerMeasure = measure.coerceIn(480, 1200)
        readerFont = if (font in setOf("sans", "serif", "mono")) font else "serif"
        val bodySize = readerSize
        val width = readerMeasure
        val face = readerFont
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    ensureVault().setReaderSettings(
                        bodySize.toUInt(),
                        width.toUInt(),
                        face,
                    )
                }
            }
        }
    }

    fun chooseReaderFont(font: String) {
        setReaderTypography(readerSize, readerMeasure, font)
    }

    /// App theme: `system`, `light` or `dark`, stored on the device.
    var themeMode by mutableStateOf(uiPreferences.getString("theme", "system") ?: "system")
        private set

    val darkTheme: Boolean?
        get() = when (themeMode) {
            "light" -> false
            "dark" -> true
            else -> null
        }

    fun chooseTheme(mode: String) {
        if (mode !in setOf("system", "light", "dark")) return
        themeMode = mode
        uiPreferences.edit().putString("theme", mode).apply()
    }

    /// Files copied by a running import/export, and whether it can still be cancelled.
    var transferProgress by mutableStateOf<Int?>(null)
        private set
    var transferBytes by mutableStateOf(0L)
        private set
    var transferLabel by mutableStateOf("")
        private set
    @Volatile
    private var transferCancelled = false

    fun cancelTransfer() {
        transferCancelled = true
    }

    /// In-note find: pattern, matches (UTF-16 ranges) and the current one.
    var findOpen by mutableStateOf(false)
        private set
    var findPattern by mutableStateOf("")
        private set
    var findRanges by mutableStateOf<List<IntRange>>(emptyList())
        private set
    var findIndex by mutableStateOf(0)
        private set
    var findReplacement by mutableStateOf("")
        private set

    val findLabel: String
        get() = if (findRanges.isEmpty()) "0/0" else "${findIndex + 1}/${findRanges.size}"

    fun openFind() {
        findOpen = true
        findPattern = ""
        findRanges = emptyList()
        findIndex = 0
    }

    fun closeFind() {
        findOpen = false
        findPattern = ""
        findReplacement = ""
        findRanges = emptyList()
        findIndex = 0
    }

    fun onFindChange(value: String) {
        findPattern = value
        refreshFind()
    }

    fun onReplaceChange(value: String) {
        findReplacement = value
    }

    /// Replace the current match with the replacement text.
    fun replaceCurrent() {
        val range = findRanges.getOrNull(findIndex) ?: return
        val text = source
        if (range.first < 0 || range.last + 1 > text.length) return
        source = text.substring(0, range.first) + findReplacement + text.substring(range.last + 1)
        dirty = true
        refreshFind()
    }

    /// Replace every match in one pass.
    fun replaceAll() {
        if (findRanges.isEmpty()) return
        val text = source
        val builder = StringBuilder()
        var last = 0
        for (range in findRanges) {
            builder.append(text, last, range.first)
            builder.append(findReplacement)
            last = range.last + 1
        }
        builder.append(text, last, text.length)
        source = builder.toString()
        dirty = true
        refreshFind()
    }

    fun findNext(backwards: Boolean = false) {
        if (findRanges.isEmpty()) return
        findIndex = if (backwards) {
            (findIndex + findRanges.size - 1) % findRanges.size
        } else {
            (findIndex + 1) % findRanges.size
        }
    }

    private fun refreshFind() {
        findRanges = if (findPattern.isEmpty()) {
            emptyList()
        } else {
            runCatching {
                findInText(source, findPattern, false, false, false)
                    .map { it.start.toInt() until it.end.toInt() }
            }.getOrDefault(emptyList())
        }
        findIndex = 0
    }

    /// Content hash of the version each open note is based on, for conflict-safe saves.
    private val fingerprints = mutableMapOf<String, ULong>()

    /// Caret position, for the debounced draft written when the app goes to the background.
    private var cursorLine = 0
    private var cursorCol = 0

    /// A draft found when the note opened, waiting for Recover / Discard.
    var pendingDraft by mutableStateOf<DraftInfo?>(null)
        private set

    /// Bumped when a recovered draft replaces the editor text.
    var recoveryNonce by mutableStateOf(0)
        private set

    fun onCursorChange(line: Int, col: Int) {
        cursorLine = line
        cursorCol = col
    }

    /// Write a draft for the open note; call from the activity's onStop.
    fun flushDraft() {
        val note = selected ?: return
        if (!dirty) return
        val text = source
        val line = cursorLine
        val col = cursorCol
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    ensureVault().saveDraft(note, text, line.toUInt(), col.toUInt())
                }
            }
        }
    }

    fun recoverDraft() {
        val draft = pendingDraft ?: return
        source = draft.text
        dirty = true
        cursorLine = draft.line.toInt()
        cursorCol = draft.col.toInt()
        pendingDraft = null
        recoveryNonce += 1
    }

    fun discardPendingDraft() {
        val note = selected ?: return
        val draft = pendingDraft
        pendingDraft = null
        if (draft != null) {
            viewModelScope.launch {
                runCatching {
                    withContext(Dispatchers.IO) { ensureVault().discardDraft(note) }
                }
            }
        }
    }

    /// Save through the shared contract: a conflict leaves the disk version alone.
    private fun saveNoteChecked(loaded: MobileVault, relative: String, text: String) {
        val report = loaded.saveDocumentWithFingerprint(relative, text, fingerprints[relative])
        when (report.status) {
            "saved" -> {
                report.fingerprint?.let { fingerprints[relative] = it.contentHash }
                loaded.discardDraft(relative)
            }
            "conflict" -> throw IllegalStateException(
                "The note changed on disk. Reload it before saving over it.",
            )
            else -> throw IllegalStateException(report.message ?: "Could not save the note")
        }
    }

    /// The shared sync state machine, fed from this vault's git status.
    val syncState: SyncStateInfo
        get() {
            val status = gitStatus
            if (status?.isRepo != true) {
                return SyncStateInfo.IDLE
            }
            return uniffi.dragonglass.syncState(
                changed = status.changed.toUInt(),
                untracked = status.untracked.toUInt(),
                ahead = status.ahead.toUInt(),
                behind = status.behind.toUInt(),
                conflicts = status.conflicts.size.toUInt(),
                busy = gitBusy,
                failed = gitError != null,
            )
        }

    /// One-line sync state for the chrome.
    val syncLabel: String
        get() {
            val status = gitStatus
            if (status?.isRepo != true) {
                return if (canSyncWithFolder) "Folder sync" else "Git sync"
            }
            return when (syncState) {
                SyncStateInfo.IDLE -> if (status.behind > 0) "Behind" else "Synced"
                SyncStateInfo.DIRTY -> "Unsynced"
                SyncStateInfo.SYNCING -> "Syncing…"
                SyncStateInfo.CONFLICT -> "Conflicts"
                SyncStateInfo.ERROR -> "Sync failed"
            }
        }

    /// Whether the sync chip should stand out.
    val syncNeedsAttention: Boolean
        get() = syncState != SyncStateInfo.IDLE

    /// Remember where the preview was scrolled when it is closed.
    fun rememberPreviewScroll(offset: Int) {
        previewScroll = offset
    }
    var previewHtml by mutableStateOf("")
        private set
    var dirty by mutableStateOf(false)
        private set
    var loading by mutableStateOf(true)
        private set
    var message by mutableStateOf<String?>(null)
        private set
    var error by mutableStateOf<String?>(null)
        private set
    var stats by mutableStateOf<StatsInfo?>(null)
        private set
    var headings by mutableStateOf<List<HeadingInfo>>(emptyList())
        private set
    var properties by mutableStateOf<List<PropertyInfo>>(emptyList())
        private set
    var relations by mutableStateOf<List<RelationInfo>>(emptyList())
        private set
    var tags by mutableStateOf<List<TagCount>>(emptyList())
        private set
    var bookmarks by mutableStateOf<Set<String>>(emptySet())
        private set
    var gitRemote by mutableStateOf("")
        private set
    var gitBranch by mutableStateOf("main")
        private set
    var gitStatus by mutableStateOf<GitStatus?>(null)
        private set
    var gitHasToken by mutableStateOf(false)
        private set
    var gitBusy by mutableStateOf(false)
        private set
    var gitMessage by mutableStateOf<String?>(null)
        private set
    var gitError by mutableStateOf<String?>(null)
        private set
    var gitConflicts by mutableStateOf<List<String>>(emptyList())
        private set

    /// An active frontmatter filter (`key` to `value`) shown above the note list.
    var propertyFilter by mutableStateOf<Pair<String, String>?>(null)
        private set

    val visibleNotes: List<NoteSummary>
        get() = (if (query.isBlank() && propertyFilter == null) notes else results)
            .filter { !bookmarksOnly || it.relative in bookmarks }

    /// Tree rows with the children of collapsed folders removed.
    val visibleTreeRows: List<TreeRow>
        get() = treeRows.filter { row ->
            collapsedDirs.none { row.path.startsWith("$it/") }
        }

    val visibleHits: List<SearchHit>
        get() = if (bookmarksOnly) hits.filter { it.relative in bookmarks } else hits

    private var searchJob: Job? = null

    init {
        loadGitSettings()
        gitHasToken = tokenStore.hasToken()
        refresh()
        refreshGitStatus()
        pullFromFolder()
    }

    private fun ensureVault(): MobileVault {
        vaultDirectory.mkdirs()
        if (vaultStore.isDefault(activeVault) && vaultDirectory.listFiles()?.isEmpty() != false) {
            File(vaultDirectory, "Welcome.md").writeText(
                """
                # Welcome to Dragon Glass

                This vault lives inside the app's private storage. Notes are plain markdown,
                so nothing is locked in.

                - Tap a note to open it.
                - Use **Edit** to change the source and **Preview** to render it.
                - The **•••** menu has the outline, details, move/rename and trash.
                - Tap the vault name at the top to import a folder from your device; imported
                  vaults stay linked and save back to that folder.

                ## What else is here

                - **Search** matches note titles and note text; tap a text match to jump to it.
                - **Tree** browses folders, and **••• → Graph view** maps the vault's links.
                - **Today** opens the daily, weekly or monthly note. **New from template** reads
                  the vault's `Templates` folder.
                - **Git sync** pushes and pulls the vault through your own GitHub token.
                """.trimIndent()
            )
        }
        return vault ?: MobileVault.open(vaultDirectory.absolutePath).also {
            vault = it
            // Reader typography is shared with the desktop reader through the vault config.
            runCatching { it.readerSettings() }.onSuccess { settings ->
                readerSize = settings.bodySize.toInt()
                readerMeasure = settings.measure.toInt()
                readerFont = settings.font
            }
        }
    }

    fun refresh() {
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    val filter = propertyFilter
                    val filtered = filter?.let { (key, value) ->
                        loaded.filterByProperty(key, value)
                    }
                    Triple(loaded.notes(), loaded.bookmarks().toSet(), loaded.treeRows()) to
                        filtered
                }
            }
            result
                .onSuccess { (core, filtered) ->
                    val (loaded, marks, rows) = core
                    notes = loaded
                    bookmarks = marks
                    treeRows = rows
                    graph = null
                    if (query.isBlank()) results = filtered ?: loaded
                }
                .onFailure { error = it.message ?: "Could not open the vault" }
            loading = false
        }
    }

    fun toggleBrowseMode() {
        browseMode = if (browseMode == BrowseMode.Tree) BrowseMode.List else BrowseMode.Tree
    }

    /// Copy a folder picked through SAF into a new app-private vault and open it.
    fun importVault(treeUri: Uri) {
        viewModelScope.launch {
            loading = true
            transferCancelled = false
            transferLabel = "Importing"
            transferProgress = 0
            val result = runCatching {
                saveDirtyNote()
                withContext(Dispatchers.IO) {
                    vaultStore.importTree(
                        treeUri,
                        onProgress = { files, bytes ->
                            transferProgress = files
                            transferBytes = bytes
                        },
                        isCancelled = { transferCancelled },
                    )
                }
            }
            result
                .onSuccess { stored ->
                    applyActiveVault(stored)
                    message = "Imported ${stored.name}"
                }
                .onFailure { failure ->
                    if (failure is VaultStore.TransferCancelled) {
                        message = "Import cancelled"
                    } else {
                        error = failure.message ?: "Could not import that folder"
                    }
                }
            transferProgress = null
            loading = false
        }
    }

    /// Write the active vault into a folder picked through SAF, merging by file name.
    fun exportVault(treeUri: Uri) {
        val active = activeVault
        viewModelScope.launch {
            loading = true
            transferCancelled = false
            transferLabel = "Exporting"
            transferProgress = 0
            val result = runCatching {
                saveDirtyNote()
                withContext(Dispatchers.IO) {
                    vaultStore.exportTree(
                        active,
                        treeUri,
                        onProgress = { files, bytes ->
                            transferProgress = files
                            transferBytes = bytes
                        },
                        isCancelled = { transferCancelled },
                    )
                }
            }
            result
                .onSuccess { message = "Exported $it ${if (it == 1) "file" else "files"}" }
                .onFailure { failure ->
                    if (failure is VaultStore.TransferCancelled) {
                        message = "Export cancelled"
                    } else {
                        error = failure.message ?: "Could not export the vault"
                    }
                }
            transferProgress = null
            loading = false
        }
    }

    /// Switch to the app-private vault or one imported from device storage.
    fun switchVault(vault: StoredVault) {
        if (vault.id == activeVault.id) return
        viewModelScope.launch {
            // Never leave a dirty note behind when the vault changes.
            val saved = runCatching { saveDirtyNote() }
            if (saved.isFailure) {
                error = saved.exceptionOrNull()?.message
                    ?: "Could not save the note; the vault was not switched"
                return@launch
            }
            vaultStore.activate(if (vaultStore.isDefault(vault)) null else vault.id)
            applyActiveVault(vaultStore.activeVault())
            message = "Opened ${vault.name}"
        }
    }

    /// Git settings are per vault; the unscoped keys keep existing setups working.
    private fun gitKey(key: String) = "${activeVault.id}.$key"

    private fun loadGitSettings() {
        gitRemote = readGit("remote", "")
        gitBranch = readGit("branch", "main").ifBlank { "main" }
    }

    /// Unix millis of the last successful sync, shown in the sync sheet.
    var lastSyncAt by mutableStateOf(readGit("lastSync", "").toLongOrNull())
        private set

    private fun writeGit(key: String, value: String) {
        gitPreferences.edit().putString(gitKey(key), value).apply()
    }

    private fun recordSync() {
        val now = System.currentTimeMillis()
        lastSyncAt = now
        writeGit("lastSync", now.toString())
    }

    private fun readGit(key: String, fallback: String): String {
        gitPreferences.getString(gitKey(key), null)?.let { return it }
        if (activeVault.id == "default") {
            return gitPreferences.getString(key, fallback).orEmpty()
        }
        return fallback
    }

    /// Copy newer files from the linked folder, then mirror local changes back.
    fun syncVault() {
        val vault = activeVault
        if (vault.treeUri == null) return
        viewModelScope.launch {
            loading = true
            transferCancelled = false
            transferLabel = "Syncing"
            transferProgress = 0
            transferBytes = 0
            val result = runCatching {
                saveDirtyNote()
                withContext(Dispatchers.IO) {
                    vaultStore.sync(
                        vault,
                        onProgress = { files, bytes ->
                            transferProgress = files
                            transferBytes = bytes
                        },
                        isCancelled = { transferCancelled },
                    )
                }
            }
            result
                .onSuccess { (pulled, written) ->
                    message = "Synced with the folder: pulled $pulled, wrote $written"
                    refresh()
                }
                .onFailure { failure ->
                    if (failure is VaultStore.TransferCancelled) {
                        message = "Sync cancelled"
                    } else {
                        error = failure.message ?: "Could not sync with the folder"
                    }
                }
            transferProgress = null
            loading = false
        }
    }

    /// Pull files that changed elsewhere whenever a linked vault is opened.
    private fun pullFromFolder() {
        val vault = activeVault
        if (vault.treeUri == null) return
        viewModelScope.launch {
            val pulled = runCatching {
                withContext(Dispatchers.IO) { vaultStore.pullNewer(vault) }
            }.getOrDefault(0)
            if (pulled > 0) {
                message = "Pulled $pulled file${if (pulled == 1) "" else "s"} from the folder"
                refresh()
            }
        }
    }

    /// Best-effort write-through after one note changes.
    private fun mirrorNote(relative: String) {
        val vault = activeVault
        if (vault.treeUri == null) return
        viewModelScope.launch {
            runCatching { withContext(Dispatchers.IO) { vaultStore.pushFile(vault, relative) } }
                .onFailure { message = "Saved on the device; the folder copy failed" }
        }
    }

    /// Best-effort mirror after a structural change (folder, rename, trash, restore).
    private fun mirrorAll() {
        val vault = activeVault
        if (vault.treeUri == null) return
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) {
                    val pulled = vaultStore.pullNewer(vault)
                    pulled to vaultStore.pushAll(vault)
                }
            }
                .onSuccess { (pulled, _) -> if (pulled > 0) refresh() }
                .onFailure { message = "Changed on the device; the folder copy failed" }
        }
    }

    /// Save the open note so switching vaults or exporting never drops an edit.
    private suspend fun saveDirtyNote() {
        val note = selected ?: return
        if (!dirty) return
        val text = source
        withContext(Dispatchers.IO) { saveNoteChecked(ensureVault(), note, text) }
        dirty = false
        mirrorNote(note)
    }

    private fun applyActiveVault(stored: StoredVault) {
        activeVault = stored
        vaultDirectory = stored.root
        vaultLabel = stored.name
        activeVaultId = stored.id
        canSyncWithFolder = stored.treeUri != null
        baseUrl = "file://${stored.root.absolutePath}/"
        vault = null
        vaults = vaultStore.vaults()
        selected = null
        source = ""
        preview = false
        previewHtml = ""
        dirty = false
        query = ""
        results = emptyList()
        hits = emptyList()
        graph = null
        trash = emptyList()
        browseMode = BrowseMode.List
        collapsedDirs = emptySet()
        gitStatus = null
        gitConflicts = emptyList()
        gitMessage = null
        gitError = null
        loadGitSettings()
        refresh()
        refreshGitStatus()
        pullFromFolder()
    }

    fun toggleBookmarksOnly() {
        bookmarksOnly = !bookmarksOnly
        if (bookmarksOnly && browseMode == BrowseMode.Tree) browseMode = BrowseMode.List
    }

    fun toggleDirectory(path: String) {
        collapsedDirs = if (path in collapsedDirs) collapsedDirs - path else collapsedDirs + path
    }

    fun onQueryChange(value: String) {
        query = value
        propertyFilter = null
        searchJob?.cancel()
        if (value.isBlank()) {
            results = notes
            hits = emptyList()
            return
        }
        searchJob = viewModelScope.launch {
            delay(120)
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    loaded.fuzzy(value, 100) to loaded.search(value, 200)
                }
            }
            result
                .onSuccess { (matched, found) ->
                    results = matched
                    hits = found
                }
                .onFailure { error = it.message ?: "Search failed" }
        }
    }

    fun openNote(relative: String) = openNote(relative, jumpToLine = null)

    /// Open a note and scroll the editor to a 1-based source line.
    fun openSearchHit(relative: String, line: Int) = openNote(relative, jumpToLine = line)

    /// Open a note from a link tapped in the preview, staying in preview mode. Unsaved
    /// edits are written first so following a link can never drop text.
    fun openLinkedNote(relative: String) {
        val current = selected
        if (current == null || !dirty) {
            openNote(relative, jumpToLine = null, keepPreview = true)
            return
        }
        val text = source
        viewModelScope.launch {
            val saved = runCatching {
                withContext(Dispatchers.IO) {
                    saveNoteChecked(ensureVault(), current, text)
                }
            }
            saved
                .onSuccess {
                    dirty = false
                    mirrorNote(current)
                }
                .onFailure { error = it.message ?: "Could not save the note" }
            if (saved.isSuccess) {
                openNote(relative, jumpToLine = null, keepPreview = true)
            }
        }
    }

    fun consumePendingLine() {
        pendingLine = null
    }

    private fun openNote(relative: String, jumpToLine: Int?, keepPreview: Boolean = false) {
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    val text = loaded.noteSource(relative)
                    OpenNote(
                        relative = relative,
                        source = text,
                        fingerprint = loaded.documentFingerprint(relative).contentHash,
                        draft = loaded.loadDraft(relative),
                        stats = documentStats(text),
                        headings = parseHeadings(text),
                        properties = parseProperties(text),
                        relations = loaded.relations(relative),
                        html = if (keepPreview) renderHtmlFromVault(text, vaultDirectory.absolutePath) else "",
                    )
                }
            }
            result
                .onSuccess { opened ->
                    opened.fingerprint?.let { fingerprints[opened.relative] = it }
                    pendingDraft = opened.draft?.takeIf { it.text != opened.source }
                    selected = opened.relative
                    source = opened.source
                    stats = opened.stats
                    headings = opened.headings
                    properties = opened.properties
                    relations = opened.relations
                    // A link followed inside the preview keeps the preview open.
                    preview = keepPreview
                    previewHtml = opened.html
                    previewScroll = 0
                    dirty = false
                    pendingLine = jumpToLine
                }
                .onFailure { error = it.message ?: "Could not open the note" }
            loading = false
        }
    }

    fun closeNote() {
        val note = selected ?: return
        viewModelScope.launch {
            if (dirty) {
                val text = source
                val saved = runCatching {
                    withContext(Dispatchers.IO) { saveNoteChecked(ensureVault(), note, text) }
                }
                if (saved.isFailure) {
                    // Keep the note open with its text: a failed save must not close it.
                    error = saved.exceptionOrNull()?.message
                        ?: "Could not save the note; it was left open"
                    return@launch
                }
                dirty = false
                mirrorNote(note)
            }
            selected = null
            source = ""
            preview = false
            previewHtml = ""
            refresh()
        }
    }

    fun onSourceChange(value: String) {
        source = value
        dirty = true
        if (findOpen) refreshFind()
    }

    fun save(showMessage: Boolean = true) {
        val note = selected ?: return
        val text = source
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    saveNoteChecked(loaded, note, text)
                    Triple(documentStats(text), parseHeadings(text), parseProperties(text))
                }
            }
            result
                .onSuccess { (newStats, newHeadings, newProperties) ->
                    stats = newStats
                    headings = newHeadings
                    properties = newProperties
                    dirty = false
                    if (showMessage) message = "Saved"
                    mirrorNote(note)
                }
                .onFailure { error = it.message ?: "Could not save the note" }
        }
    }

    fun togglePreview() {
        if (preview) {
            preview = false
            return
        }
        val text = source
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) { renderHtmlFromVault(text, vaultDirectory.absolutePath) }
            }
            result
                .onSuccess {
                    previewHtml = it
                    preview = true
                }
                .onFailure { error = it.message ?: "Could not render the note" }
        }
    }

    fun newNote() {
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    var index = 1
                    var relative = "Untitled.md"
                    while (File(vaultDirectory, relative).exists()) {
                        index++
                        relative = "Untitled $index.md"
                    }
                    loaded.createNote(relative, "# Untitled\n\n")
                    relative
                }
            }
            result
                .onSuccess {
                    refresh()
                    openNote(it)
                    mirrorNote(it)
                }
                .onFailure { error = it.message ?: "Could not create the note" }
        }
    }

    /// Create a note from a typed title: the title becomes the file name and the heading.
    /// `folder` places it in a subfolder (created when missing); empty means the vault root.
    fun createNote(title: String, folder: String = "") {
        val trimmed = title.trim()
        if (trimmed.isEmpty()) {
            error = "A note needs a name"
            return
        }
        val safe = trimmed
            .map { ch -> if (ch in "/\\:*?\"<>|") '-' else ch }
            .joinToString("")
            .trim()
        if (safe.isEmpty()) {
            error = "A note needs a name"
            return
        }
        val folderPath = folder.trim().trim('/').trim()
        if (folderPath.split('/').any { it == ".." }) {
            error = "A folder name cannot contain .."
            return
        }
        val prefix = if (folderPath.isEmpty()) "" else "$folderPath/"
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    var relative = "$prefix$safe.md"
                    var index = 1
                    while (File(vaultDirectory, relative).exists()) {
                        index++
                        relative = "$prefix$safe $index.md"
                    }
                    loaded.createNote(relative, "# $trimmed\n\n")
                    relative
                }
            }
            result
                .onSuccess {
                    refresh()
                    openNote(it)
                    mirrorNote(it)
                }
                .onFailure { error = it.message ?: "Could not create the note" }
        }
    }

    fun openDaily() = openPeriodic("daily") { it.openDaily() }

    fun openWeekly() = openPeriodic("weekly") { it.openWeekly() }

    fun openMonthly() = openPeriodic("monthly") { it.openMonthly() }

    private fun openPeriodic(label: String, open: (MobileVault) -> String) {
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) { open(ensureVault()) }
            }
            result
                .onSuccess {
                    refresh()
                    openNote(it)
                    mirrorNote(it)
                }
                .onFailure { error = it.message ?: "Could not open the $label note" }
        }
    }

    fun loadTemplates() {
        viewModelScope.launch {
            runCatching { withContext(Dispatchers.IO) { ensureVault().templates() } }
                .onSuccess { templates = it }
                .onFailure { error = it.message ?: "Could not load templates" }
        }
    }

    fun createFromTemplate(template: String, title: String) {
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) { ensureVault().createFromTemplate(template, title) }
            }
            result
                .onSuccess {
                    refresh()
                    openNote(it)
                    mirrorNote(it)
                }
                .onFailure { error = it.message ?: "Could not create the note" }
            loading = false
        }
    }

    fun loadGraph() {
        if (graphLoading) return
        viewModelScope.launch {
            graphLoading = true
            runCatching { withContext(Dispatchers.IO) { ensureVault().graph() } }
                .onSuccess { graph = it }
                .onFailure { error = it.message ?: "Could not build the graph" }
            graphLoading = false
        }
    }

    fun deleteSelected() {
        val note = selected ?: return
        val text = source
        val wasDirty = dirty
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    if (wasDirty) saveNoteChecked(loaded, note, text)
                    loaded.trashNote(note)
                }
            }
            result
                .onSuccess {
                    selected = null
                    source = ""
                    preview = false
                    dirty = false
                    message = "Moved $note to the trash"
                    refresh()
                    mirrorAll()
                }
                .onFailure { error = it.message ?: "Could not move the note to the trash" }
        }
    }

    /// Close the open note (saving first) and list the notes whose property matches.
    fun filterByProperty(key: String, value: String) {
        val note = selected
        val text = source
        val dirtyNow = dirty
        viewModelScope.launch {
            if (note != null && dirtyNow) {
                runCatching { withContext(Dispatchers.IO) { saveNoteChecked(ensureVault(), note, text) } }
                    .onFailure { error = it.message ?: "Could not save the note" }
            }
            selected = null
            source = ""
            preview = false
            previewScroll = 0
            dirty = false
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) { ensureVault().filterByProperty(key, value) }
            }
            result
                .onSuccess { rows ->
                    propertyFilter = key to value
                    query = ""
                    results = rows
                    hits = emptyList()
                }
                .onFailure { error = it.message ?: "Could not filter the notes" }
            loading = false
        }
    }

    fun clearPropertyFilter() {
        propertyFilter = null
        refresh()
    }

    /// Follow a relation chip: resolve its target through the engine's index and open it.
    fun openRelation(target: String) {
        val wanted = target.substringBefore('#').trim()
        if (wanted.isEmpty()) return
        viewModelScope.launch {
            val relative = runCatching {
                withContext(Dispatchers.IO) { ensureVault().resolveTarget(wanted) }
            }.getOrNull()
            if (relative != null) {
                openLinkedNote(relative)
            } else {
                error = "No note matches $target"
            }
        }
    }

    /// Close the open note and search the vault for a tag, so Details chips can filter.
    fun filterByTag(tag: String) {
        val note = selected
        val text = source
        val dirtyNow = dirty
        viewModelScope.launch {
            if (note != null && dirtyNow) {
                runCatching { withContext(Dispatchers.IO) { saveNoteChecked(ensureVault(), note, text) } }
                    .onFailure { error = it.message ?: "Could not save the note" }
            }
            selected = null
            source = ""
            preview = false
            previewScroll = 0
            dirty = false
            onQueryChange(tag)
        }
    }

    /// Move any note from the list to the trash, saving it first when it is the open one.
    fun trashNote(relative: String) {
        val text = source
        val isOpen = selected == relative
        val saveFirst = isOpen && dirty
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    if (saveFirst) saveNoteChecked(loaded, relative, text)
                    loaded.trashNote(relative)
                }
            }
            result
                .onSuccess {
                    if (isOpen) {
                        selected = null
                        source = ""
                        preview = false
                        dirty = false
                    }
                    message = "Moved $relative to the trash"
                    refresh()
                    mirrorAll()
                }
                .onFailure { error = it.message ?: "Could not move the note to the trash" }
        }
    }

    /// Rename or move any note from the list.
    fun renameNote(relative: String, destination: String) {
        val trimmed = destination.trim().trimStart('/')
        if (trimmed.isEmpty()) {
            error = "Enter a file name"
            return
        }
        val target = if (trimmed.substringAfterLast('/').contains('.')) {
            trimmed
        } else {
            "$trimmed.md"
        }
        val text = source
        val isOpen = selected == relative
        val saveFirst = isOpen && dirty
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    if (saveFirst) saveNoteChecked(loaded, relative, text)
                    loaded.renameEntry(relative, target)
                }
            }
            result
                .onSuccess { renamed ->
                    message = "Moved to $renamed"
                    refresh()
                    if (isOpen) openNote(renamed)
                    mirrorAll()
                }
                .onFailure { error = it.message ?: "Could not move the note" }
            loading = false
        }
    }

    /// Rename or move the open note, saving unsaved edits first.
    fun renameSelected(destination: String) {
        val note = selected ?: return
        val text = source
        val wasDirty = dirty
        val trimmed = destination.trim().trimStart('/')
        if (trimmed.isEmpty()) {
            error = "Enter a file name"
            return
        }
        val target = if (trimmed.substringAfterLast('/').contains('.')) {
            trimmed
        } else {
            "$trimmed.md"
        }
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    if (wasDirty) saveNoteChecked(loaded, note, text)
                    loaded.renameEntry(note, target)
                }
            }
            result
                .onSuccess { renamed ->
                    message = "Moved to $renamed"
                    refresh()
                    openNote(renamed)
                    mirrorAll()
                }
                .onFailure { error = it.message ?: "Could not move the note" }
            loading = false
        }
    }

    fun createFolder(relative: String) {
        val trimmed = relative.trim().trim('/')
        if (trimmed.isEmpty()) {
            error = "Enter a folder name"
            return
        }
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) { ensureVault().createFolder(trimmed) }
            }
            result
                .onSuccess {
                    browseMode = BrowseMode.Tree
                    message = "Created $trimmed"
                    refresh()
                    mirrorAll()
                }
                .onFailure { error = it.message ?: "Could not create the folder" }
        }
    }

    fun loadTrash() {
        if (trashLoading) return
        viewModelScope.launch {
            trashLoading = true
            runCatching { withContext(Dispatchers.IO) { ensureVault().trashEntries() } }
                .onSuccess { trash = it }
                .onFailure { error = it.message ?: "Could not read the trash" }
            trashLoading = false
        }
    }

    fun restoreTrashEntry(trashRelative: String) {
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) { ensureVault().restoreTrashEntry(trashRelative) }
            }
            result
                .onSuccess {
                    message = "Restored $it"
                    refresh()
                    loadTrash()
                    mirrorAll()
                }
                .onFailure { error = it.message ?: "Could not restore the entry" }
        }
    }

    fun emptyTrash() {
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) { ensureVault().emptyTrash() }
            }
            result
                .onSuccess { removed ->
                    trash = emptyList()
                    message = if (removed == 1L) "Removed 1 entry" else "Removed $removed entries"
                }
                .onFailure { error = it.message ?: "Could not empty the trash" }
        }
    }

    fun loadTags() {
        viewModelScope.launch {
            runCatching { withContext(Dispatchers.IO) { ensureVault().tags() } }
                .onSuccess { tags = it }
                .onFailure { error = it.message ?: "Could not load tags" }
        }
    }

    fun toggleBookmark(relative: String) {
        viewModelScope.launch {
            runCatching {
                withContext(Dispatchers.IO) { ensureVault().toggleBookmark(relative) }
            }
                .onSuccess { bookmarked ->
                    bookmarks = if (bookmarked) bookmarks + relative else bookmarks - relative
                }
                .onFailure { error = it.message ?: "Could not update the bookmark" }
        }
    }

    fun refreshGitStatus() {
        val remote = gitRemote
        val branch = gitBranch
        viewModelScope.launch {
            val status = runCatching {
                withContext(Dispatchers.IO) {
                    GitSyncManager(vaultDirectory, remote, branch, tokenStore.load()).status()
                }
            }.getOrNull()
            if (status != null) gitStatus = status
        }
    }

    fun saveGitSettings(remote: String, branch: String, token: String?) {
        val cleanRemote = remote.trim()
        val cleanBranch = branch.trim().ifBlank { "main" }
        gitRemote = cleanRemote
        gitBranch = cleanBranch
        gitPreferences.edit()
            .putString(gitKey("remote"), cleanRemote)
            .putString(gitKey("branch"), cleanBranch)
            .apply()
        if (!token.isNullOrBlank()) {
            tokenStore.save(token.trim())
            gitHasToken = true
        }
        gitError = null
        refreshGitStatus()
    }

    fun syncGit() {
        val remote = gitRemote
        val branch = gitBranch
        val token = tokenStore.load()
        if (remote.isBlank()) {
            gitError = "Set the GitHub remote URL first"
            return
        }
        if (token == null && !remote.startsWith("file:")) {
            gitError = "Add a GitHub token first"
            return
        }
        viewModelScope.launch {
            gitBusy = true
            gitError = null
            gitMessage = null
            gitConflicts = emptyList()
            // Commit what the user sees, and do not sync (or close the note) if that fails.
            val saved = runCatching { saveDirtyNote() }
            if (saved.isFailure) {
                gitError = saved.exceptionOrNull()?.message
                    ?: "Could not save the open note; git sync did not run"
                gitBusy = false
                return@launch
            }
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    GitSyncManager(vaultDirectory, remote, branch, token).sync()
                }
            }
            result
                .onSuccess { synced ->
                    gitStatus = synced.status
                    recordSync()
                    gitMessage = buildString {
                        if (synced.committed) append("Committed local changes. ")
                        if (synced.pulled) append("Pulled remote changes. ")
                        append("Pushed.")
                    }
                    selected = null
                    source = ""
                    preview = false
                    dirty = false
                    refresh()
                }
                .onFailure { failure ->
                    if (failure is GitSyncException) gitConflicts = failure.conflicts
                    gitError = gitErrorMessage(failure)
                    refreshGitStatus()
                }
            gitBusy = false
        }
    }

    fun initGitRepository() {
        val remote = gitRemote
        val branch = gitBranch
        val token = tokenStore.load()
        if (remote.isBlank()) {
            gitError = "Set the GitHub remote URL first"
            return
        }
        if (token == null && !remote.startsWith("file:")) {
            gitError = "Add a GitHub token first"
            return
        }
        viewModelScope.launch {
            gitBusy = true
            gitError = null
            gitMessage = null
            gitConflicts = emptyList()
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    GitSyncManager(vaultDirectory, remote, branch, token).initRemote()
                }
            }
            result
                .onSuccess {
                    gitStatus = it
                    gitMessage = "Repository initialized and first commit pushed."
                    refresh()
                }
                .onFailure { failure ->
                    if (failure is GitSyncException) gitConflicts = failure.conflicts
                    gitError = gitErrorMessage(failure)
                    refreshGitStatus()
                }
            gitBusy = false
        }
    }

    fun cloneGitRepository() {
        val remote = gitRemote
        val branch = gitBranch
        val token = tokenStore.load()
        if (remote.isBlank()) {
            gitError = "Set the GitHub remote URL first"
            return
        }
        if (token == null && !remote.startsWith("file:")) {
            gitError = "Add a GitHub token first"
            return
        }
        viewModelScope.launch {
            gitBusy = true
            gitError = null
            gitMessage = null
            gitConflicts = emptyList()
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val manager = GitSyncManager(vaultDirectory, remote, branch, token)
                    val status = manager.cloneIntoVault()
                    vault?.refresh()
                    status
                }
            }
            result
                .onSuccess {
                    selected = null
                    source = ""
                    preview = false
                    dirty = false
                    gitStatus = it
                    gitMessage = "Linked the GitHub repository. The previous vault was kept as a backup."
                    refresh()
                }
                .onFailure { failure ->
                    gitError = gitErrorMessage(failure)
                    refreshGitStatus()
                }
            gitBusy = false
        }
    }

    fun clearGitMessages() {
        gitError = null
        gitMessage = null
        gitConflicts = emptyList()
    }

    fun clearGitToken() {
        tokenStore.clear()
        gitHasToken = false
        gitMessage = "GitHub token removed."
    }

    fun abortGitRebase() {
        val remote = gitRemote
        val branch = gitBranch
        viewModelScope.launch {
            gitBusy = true
            runCatching {
                withContext(Dispatchers.IO) {
                    GitSyncManager(vaultDirectory, remote, branch, tokenStore.load()).abortRebase()
                }
            }.onSuccess {
                gitConflicts = emptyList()
                gitError = null
                gitMessage = "Rebase aborted; local notes were kept."
                refreshGitStatus()
            }.onFailure {
                gitError = gitErrorMessage(it)
            }
            gitBusy = false
        }
    }

    private fun gitErrorMessage(error: Throwable): String {
        val raw = error.message?.takeIf { it.isNotBlank() } ?: "Git operation failed"
        val lower = raw.lowercase()
        return when {
            lower.contains("not authorized") ||
                lower.contains("authentication") ||
                lower.contains("401") ->
                "GitHub rejected the token. Check that it is valid, not expired, and has " +
                    "Contents: Read and write for this repository."
            lower.contains("receive-pack") ||
                lower.contains("not permitted") ||
                lower.contains("403") ||
                lower.contains("forbidden") ->
                "GitHub refused the push. The token can read this repository but not write " +
                    "to it: give it Contents: Read and write and make sure the repository is " +
                    "included in the token's access."
            lower.contains("not found") || lower.contains("404") ->
                "Repository not found. Check the remote URL and that the token can access it."
            else -> raw
        }
    }

    /// Character offset of a 1-based source line, for jumping from the outline.
    fun offsetForLine(line: Int): Int {
        val target = (line - 1).coerceAtLeast(0)
        val lines = source.split('\n')
        var offset = 0
        for (index in 0 until minOf(target, lines.size)) {
            offset += lines[index].length + 1
        }
        return offset.coerceIn(0, source.length)
    }

    fun clearMessage() {
        message = null
    }

    fun clearError() {
        error = null
    }
}
