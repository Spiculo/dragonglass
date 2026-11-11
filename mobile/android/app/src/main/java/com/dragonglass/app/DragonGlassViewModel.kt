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
import uniffi.dragonglass.GraphView
import uniffi.dragonglass.HeadingInfo
import uniffi.dragonglass.MobileVault
import uniffi.dragonglass.NoteSummary
import uniffi.dragonglass.PropertyInfo
import uniffi.dragonglass.RelationInfo
import uniffi.dragonglass.SearchHit
import uniffi.dragonglass.StatsInfo
import uniffi.dragonglass.TagCount
import uniffi.dragonglass.TemplateInfo
import uniffi.dragonglass.TrashEntry
import uniffi.dragonglass.TreeRow
import uniffi.dragonglass.documentStats
import uniffi.dragonglass.headings as parseHeadings
import uniffi.dragonglass.properties as parseProperties
import uniffi.dragonglass.renderHtml
import java.io.File

private data class OpenNote(
    val relative: String,
    val source: String,
    val stats: StatsInfo,
    val headings: List<HeadingInfo>,
    val properties: List<PropertyInfo>,
    val relations: List<RelationInfo>,
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
        private set
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

    val visibleNotes: List<NoteSummary>
        get() = (if (query.isBlank()) notes else results)
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
        return vault ?: MobileVault.open(vaultDirectory.absolutePath).also { vault = it }
    }

    fun refresh() {
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    Triple(loaded.notes(), loaded.bookmarks().toSet(), loaded.treeRows())
                }
            }
            result
                .onSuccess { (loaded, marks, rows) ->
                    notes = loaded
                    bookmarks = marks
                    treeRows = rows
                    graph = null
                    if (query.isBlank()) results = loaded
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
            val result = runCatching {
                saveDirtyNote()
                withContext(Dispatchers.IO) { vaultStore.importTree(treeUri) }
            }
            result
                .onSuccess { stored ->
                    applyActiveVault(stored)
                    message = "Imported ${stored.name}"
                }
                .onFailure { error = it.message ?: "Could not import that folder" }
            loading = false
        }
    }

    /// Write the active vault into a folder picked through SAF, merging by file name.
    fun exportVault(treeUri: Uri) {
        val active = activeVault
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                saveDirtyNote()
                withContext(Dispatchers.IO) { vaultStore.exportTree(active, treeUri) }
            }
            result
                .onSuccess { message = "Exported $it ${if (it == 1) "file" else "files"}" }
                .onFailure { error = it.message ?: "Could not export the vault" }
            loading = false
        }
    }

    /// Switch to the app-private vault or one imported from device storage.
    fun switchVault(vault: StoredVault) {
        if (vault.id == activeVault.id) return
        viewModelScope.launch {
            runCatching { saveDirtyNote() }
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
            val result = runCatching {
                saveDirtyNote()
                withContext(Dispatchers.IO) { vaultStore.sync(vault) }
            }
            result
                .onSuccess { (pulled, written) ->
                    message = "Synced with the folder: pulled $pulled, wrote $written"
                    refresh()
                }
                .onFailure { error = it.message ?: "Could not sync with the folder" }
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
        withContext(Dispatchers.IO) { ensureVault().saveNote(note, text) }
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

    fun consumePendingLine() {
        pendingLine = null
    }

    private fun openNote(relative: String, jumpToLine: Int?) {
        viewModelScope.launch {
            loading = true
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    val text = loaded.noteSource(relative)
                    OpenNote(
                        relative = relative,
                        source = text,
                        stats = documentStats(text),
                        headings = parseHeadings(text),
                        properties = parseProperties(text),
                        relations = loaded.relations(relative),
                    )
                }
            }
            result
                .onSuccess { opened ->
                    selected = opened.relative
                    source = opened.source
                    stats = opened.stats
                    headings = opened.headings
                    properties = opened.properties
                    relations = opened.relations
                    preview = false
                    previewHtml = ""
                    dirty = false
                    pendingLine = jumpToLine
                }
                .onFailure { error = it.message ?: "Could not open the note" }
            loading = false
        }
    }

    fun closeNote() {
        val note = selected
        viewModelScope.launch {
            if (note != null && dirty) {
                runCatching { withContext(Dispatchers.IO) { ensureVault().saveNote(note, source) } }
                    .onFailure { error = it.message ?: "Could not save the note" }
                mirrorNote(note)
            }
            selected = null
            source = ""
            preview = false
            previewHtml = ""
            dirty = false
            refresh()
        }
    }

    fun onSourceChange(value: String) {
        source = value
        dirty = true
    }

    fun save(showMessage: Boolean = true) {
        val note = selected ?: return
        val text = source
        viewModelScope.launch {
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    val loaded = ensureVault()
                    loaded.saveNote(note, text)
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
                withContext(Dispatchers.IO) { renderHtml(text) }
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
                    if (wasDirty) loaded.saveNote(note, text)
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
                    if (wasDirty) loaded.saveNote(note, text)
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
            val result = runCatching {
                withContext(Dispatchers.IO) {
                    GitSyncManager(vaultDirectory, remote, branch, token).sync()
                }
            }
            result
                .onSuccess { synced ->
                    gitStatus = synced.status
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
