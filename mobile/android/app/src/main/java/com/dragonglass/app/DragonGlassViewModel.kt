package com.dragonglass.app

import android.app.Application
import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
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
    private val vaultDirectory = File(application.filesDir, "vault")
    private val gitPreferences =
        application.getSharedPreferences("dragon_glass_git", Context.MODE_PRIVATE)
    private val tokenStore = SecureTokenStore(application)
    private var vault: MobileVault? = null

    val vaultPath: String = vaultDirectory.absolutePath
    val baseUrl: String = "file://${vaultDirectory.absolutePath}/"
    val vaultLabel: String = "On this device"

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
        gitRemote = gitPreferences.getString("remote", "").orEmpty()
        gitBranch = gitPreferences.getString("branch", "main").orEmpty().ifBlank { "main" }
        gitHasToken = tokenStore.hasToken()
        refresh()
        refreshGitStatus()
    }

    private fun ensureVault(): MobileVault {
        vaultDirectory.mkdirs()
        if (vaultDirectory.listFiles()?.isEmpty() != false) {
            File(vaultDirectory, "Welcome.md").writeText(
                """
                # Welcome to Dragon Glass

                This vault lives inside the app's private storage. Notes are plain markdown,
                so nothing is locked in.

                - Tap a note to open it.
                - Use **Edit** to change the source and **Preview** to render it.
                - The “More” menu has the outline, document details and deletion.

                ## Next steps

                Import an existing vault through Android's document picker once the storage
                layer lands. The shared Rust engine already parses notes, links, math and
                properties; this app is the Android shell around it.
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
            .putString("remote", cleanRemote)
            .putString("branch", cleanBranch)
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
