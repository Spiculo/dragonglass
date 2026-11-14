package com.dragonglass.app.ui

import android.content.Intent
import android.net.Uri
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import com.dragonglass.app.BrowseMode
import com.dragonglass.app.DragonGlassViewModel
import com.dragonglass.app.R
import com.dragonglass.app.storage.StoredVault
import com.dragonglass.app.sync.GitStatus
import uniffi.dragonglass.GraphView
import uniffi.dragonglass.HeadingInfo
import uniffi.dragonglass.NoteSummary
import uniffi.dragonglass.SearchHit
import uniffi.dragonglass.TreeRow
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.min
import kotlin.math.sin
import kotlin.math.sqrt

@Composable
fun DragonGlassApp(vm: DragonGlassViewModel) {
    val snackbar = remember { SnackbarHostState() }

    val message = vm.message
    LaunchedEffect(message) {
        if (message != null) {
            snackbar.showSnackbar(message)
            vm.clearMessage()
        }
    }
    val error = vm.error
    LaunchedEffect(error) {
        if (error != null) {
            snackbar.showSnackbar(error)
            vm.clearError()
        }
    }

    // The note lists stay in these states across a note opening and closing, so Back
    // returns to the same row instead of the top of the list.
    val noteListState = rememberLazyListState()
    val treeListState = rememberLazyListState()
    val searchListState = rememberLazyListState()

    Box(Modifier.fillMaxSize()) {
        val selected = vm.selected
        if (selected == null) {
            NotesScreen(vm, noteListState, treeListState, searchListState)
        } else {
            EditorScreen(vm, selected)
        }

        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .navigationBarsPadding()
                .padding(16.dp),
        )

        if (vm.loading) {
            LinearProgressIndicator(
                modifier = Modifier
                    .fillMaxWidth()
                    .align(Alignment.TopCenter)
                    .statusBarsPadding(),
            )
        }
    }
}

private enum class VaultAction { Import, Export }

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NotesScreen(
    vm: DragonGlassViewModel,
    noteListState: LazyListState,
    treeListState: LazyListState,
    searchListState: LazyListState,
) {
    var showSync by remember { mutableStateOf(false) }
    var showGraph by remember { mutableStateOf(false) }
    var showTemplates by remember { mutableStateOf(false) }
    var showTrash by remember { mutableStateOf(false) }
    var showNewFolder by remember { mutableStateOf(false) }
    var showNewNote by remember { mutableStateOf(false) }
    var actionNote by remember { mutableStateOf<NoteSummary?>(null) }
    var renamingNote by remember { mutableStateOf<NoteSummary?>(null) }
    val openRowActions: (NoteSummary) -> Unit = { actionNote = it }
    var showVaults by remember { mutableStateOf(false) }
    var pendingAction by remember { mutableStateOf<VaultAction?>(null) }
    var menuOpen by remember { mutableStateOf(false) }
    var todayOpen by remember { mutableStateOf(false) }
    val focusManager = LocalFocusManager.current

    // Back clears the search before it leaves the screen, so a query is never lost by
    // accident and never traps the user either.
    BackHandler(enabled = vm.query.isNotEmpty() || vm.bookmarksOnly) {
        if (vm.query.isNotEmpty()) {
            vm.onQueryChange("")
        } else {
            vm.toggleBookmarksOnly()
        }
    }

    val vaultPicker = rememberLauncherForActivityResult(
        ActivityResultContracts.OpenDocumentTree(),
    ) { uri ->
        val action = pendingAction
        pendingAction = null
        showVaults = false
        if (uri != null) {
            when (action) {
                VaultAction.Import -> vm.importVault(uri)
                VaultAction.Export -> vm.exportVault(uri)
                null -> Unit
            }
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier.clickable { showVaults = true },
                    ) {
                        Image(
                            painter = painterResource(R.drawable.app_logo),
                            contentDescription = null,
                            modifier = Modifier.size(34.dp),
                        )
                        Spacer(Modifier.width(10.dp))
                        Column {
                            Text("Dragon Glass", fontWeight = FontWeight.SemiBold)
                            Text(
                                text = "${vm.vaultLabel} · tap to switch",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                },
                actions = {
                    SyncChip(vm) {
                        vm.clearGitMessages()
                        vm.refreshGitStatus()
                        showSync = true
                    }
                    Box {
                        TextButton(onClick = { todayOpen = true }) { Text("Today") }
                        DropdownMenu(
                            expanded = todayOpen,
                            onDismissRequest = { todayOpen = false },
                        ) {
                            DropdownMenuItem(
                                text = { Text("Daily note") },
                                onClick = {
                                    todayOpen = false
                                    vm.openDaily()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Weekly note") },
                                onClick = {
                                    todayOpen = false
                                    vm.openWeekly()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Monthly note") },
                                onClick = {
                                    todayOpen = false
                                    vm.openMonthly()
                                },
                            )
                        }
                    }
                    Box {
                        TextButton(onClick = { menuOpen = true }) { Text("•••") }
                        DropdownMenu(
                            expanded = menuOpen,
                            onDismissRequest = { menuOpen = false },
                        ) {
                            DropdownMenuItem(
                                text = { Text("Graph view") },
                                onClick = {
                                    menuOpen = false
                                    vm.loadGraph()
                                    showGraph = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("New from template") },
                                onClick = {
                                    menuOpen = false
                                    showTemplates = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("New folder…") },
                                onClick = {
                                    menuOpen = false
                                    showNewFolder = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Trash") },
                                onClick = {
                                    menuOpen = false
                                    showTrash = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Vaults and storage…") },
                                onClick = {
                                    menuOpen = false
                                    showVaults = true
                                },
                            )
                            DropdownMenuItem(
                                text = {
                                    Text(
                                        if (vm.bookmarksOnly) "Show all notes" else "Bookmarks only",
                                    )
                                },
                                onClick = {
                                    menuOpen = false
                                    vm.toggleBookmarksOnly()
                                },
                            )
                            HorizontalDivider()
                            DropdownMenuItem(
                                text = { Text("Git sync") },
                                onClick = {
                                    menuOpen = false
                                    vm.clearGitMessages()
                                    vm.refreshGitStatus()
                                    showSync = true
                                },
                            )
                        }
                    }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = { showNewNote = true },
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ) {
                Text("+", fontSize = 26.sp)
            }
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = vm.query,
                    onValueChange = vm::onQueryChange,
                    placeholder = {
                        Text(if (vm.bookmarksOnly) "Search bookmarks" else "Search notes")
                    },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { focusManager.clearFocus() }),
                    trailingIcon = {
                        if (vm.query.isNotEmpty()) {
                            TextButton(onClick = { vm.onQueryChange("") }) { Text("Clear") }
                        }
                    },
                    modifier = Modifier.weight(1f),
                )
                if (vm.bookmarksOnly) {
                    TextButton(onClick = vm::toggleBookmarksOnly) { Text("All") }
                } else if (vm.query.isBlank()) {
                    TextButton(onClick = vm::toggleBrowseMode) {
                        Text(if (vm.browseMode == BrowseMode.Tree) "List" else "Tree")
                    }
                }
            }

            when {
                vm.query.isNotBlank() -> SearchResults(vm, searchListState, openRowActions)
                vm.notes.isEmpty() && !vm.loading -> EmptyVault()
                vm.browseMode == BrowseMode.Tree -> TreeList(vm, treeListState, openRowActions)
                vm.visibleNotes.isEmpty() -> MutedLine(
                    if (vm.bookmarksOnly) "No bookmarked notes yet." else "No notes here yet.",
                )
                else -> NoteList(vm, noteListState, openRowActions)
            }
        }
    }

    if (showSync) {
        GitSyncSheet(vm, onDismiss = { showSync = false })
    }
    if (showGraph) {
        GraphSheet(vm, onDismiss = { showGraph = false })
    }
    if (showTemplates) {
        TemplatesSheet(
            vm = vm,
            onDismiss = { showTemplates = false },
            onCreated = { showTemplates = false },
        )
    }
    if (showTrash) {
        TrashSheet(vm, onDismiss = { showTrash = false })
    }
    val target = actionNote
    if (target != null) {
        ModalBottomSheet(onDismissRequest = { actionNote = null }) {
            Column(Modifier.navigationBarsPadding()) {
                ListItem(
                    headlineContent = { Text(target.title, maxLines = 1) },
                    supportingContent = { Text(target.relative, style = MaterialTheme.typography.labelSmall) },
                )
                HorizontalDivider()
                TextButton(
                    onClick = {
                        vm.toggleBookmark(target.relative)
                        actionNote = null
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        if (target.relative in vm.bookmarks) "Remove bookmark" else "Bookmark",
                    )
                }
                TextButton(
                    onClick = {
                        actionNote = null
                        renamingNote = target
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Rename or move…") }
                TextButton(
                    onClick = {
                        actionNote = null
                        vm.trashNote(target.relative)
                    },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("Move to trash") }
            }
        }
    }

    val renameTarget = renamingNote
    if (renameTarget != null) {
        TextInputDialog(
            title = "Rename or move",
            label = "Path",
            initial = renameTarget.relative,
            hint = "Use a folder prefix to move it, e.g. notes/New name.",
            confirmLabel = "Rename",
            onDismiss = { renamingNote = null },
            onConfirm = {
                vm.renameNote(renameTarget.relative, it)
                renamingNote = null
            },
        )
    }
    if (showNewNote) {
        TextInputDialog(
            title = "New note",
            label = "Title",
            initial = "",
            hint = "The title becomes the file name and the first heading.",
            confirmLabel = "Create",
            onDismiss = { showNewNote = false },
            onConfirm = {
                vm.createNote(it)
                showNewNote = false
            },
        )
    }
    if (showNewFolder) {
        TextInputDialog(
            title = "New folder",
            label = "Folder path",
            initial = "",
            hint = "Nested folders are fine, e.g. notes/projects.",
            confirmLabel = "Create",
            onDismiss = { showNewFolder = false },
            onConfirm = {
                vm.createFolder(it)
                showNewFolder = false
            },
        )
    }
    if (showVaults) {
        VaultsSheet(
            vm = vm,
            onDismiss = { showVaults = false },
            onImport = {
                showVaults = false
                pendingAction = VaultAction.Import
                vaultPicker.launch(null)
            },
            onExport = {
                showVaults = false
                pendingAction = VaultAction.Export
                vaultPicker.launch(null)
            },
        )
    }
}

@Composable
private fun NoteList(
    vm: DragonGlassViewModel,
    state: LazyListState,
    onRowAction: (NoteSummary) -> Unit,
) {
    LazyColumn(state = state, modifier = Modifier.fillMaxSize()) {
        items(vm.visibleNotes, key = { it.relative }) { note ->
            NoteRow(
                note = note,
                bookmarked = note.relative in vm.bookmarks,
                onClick = { vm.openNote(note.relative) },
                onToggleBookmark = { vm.toggleBookmark(note.relative) },
                onLongClick = { onRowAction(note) },
            )
            HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.25f))
        }
    }
}

@Composable
private fun TreeList(
    vm: DragonGlassViewModel,
    state: LazyListState,
    onRowAction: (NoteSummary) -> Unit,
) {
    LazyColumn(state = state, modifier = Modifier.fillMaxSize()) {
        items(vm.visibleTreeRows, key = { it.path }) { row ->
            if (row.isDir) {
                DirectoryRow(
                    row = row,
                    collapsed = row.path in vm.collapsedDirs,
                    onClick = { vm.toggleDirectory(row.path) },
                )
            } else {
                NoteRow(
                    note = NoteSummary(relative = row.path, title = row.name),
                    bookmarked = row.path in vm.bookmarks,
                    depth = row.depth,
                    onClick = { vm.openNote(row.path) },
                    onToggleBookmark = { vm.toggleBookmark(row.path) },
                    onLongClick = {
                        onRowAction(NoteSummary(relative = row.path, title = row.name))
                    },
                )
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.2f))
        }
    }
}

@Composable
private fun DirectoryRow(row: TreeRow, collapsed: Boolean, onClick: () -> Unit) {
    ListItem(
        headlineContent = {
            Text(
                text = (if (collapsed) "▸ " else "▾ ") + row.name,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                fontWeight = FontWeight.Medium,
            )
        },
        modifier = Modifier
            .clickable(onClick = onClick)
            .padding(start = (row.depth * 16).dp),
    )
}

@Composable
private fun SearchResults(
    vm: DragonGlassViewModel,
    state: LazyListState,
    onRowAction: (NoteSummary) -> Unit,
) {
    if (vm.visibleNotes.isEmpty() && vm.visibleHits.isEmpty() && !vm.loading) {
        MutedLine("No notes or text match “${vm.query}”.")
        return
    }
    LazyColumn(state = state, modifier = Modifier.fillMaxSize()) {
        if (vm.visibleNotes.isNotEmpty()) {
            item { SectionHeader("Notes") }
            items(vm.visibleNotes, key = { "note:${it.relative}" }) { note ->
                NoteRow(
                    note = note,
                    bookmarked = note.relative in vm.bookmarks,
                    onClick = { vm.openNote(note.relative) },
                    onToggleBookmark = { vm.toggleBookmark(note.relative) },
                    onLongClick = { onRowAction(note) },
                )
                HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.2f))
            }
        }
        if (vm.visibleHits.isNotEmpty()) {
            item { SectionHeader("Text matches") }
            items(vm.visibleHits, key = { "hit:${it.relative}:${it.line}" }) { hit ->
                SearchHitRow(hit) { vm.openSearchHit(hit.relative, hit.line) }
                HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.2f))
            }
        }
    }
}

@Composable
private fun SearchHitRow(hit: SearchHit, onClick: () -> Unit) {
    ListItem(
        headlineContent = {
            Text(
                text = hit.text.trim().ifBlank { "(empty line)" },
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = {
            Text(
                text = "${hit.relative} · line ${hit.line}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        modifier = Modifier.clickable(onClick = onClick),
    )
}

/// A compact sync state, tappable to open the sync sheet.
@Composable
private fun SyncChip(vm: DragonGlassViewModel, onClick: () -> Unit) {
    val color = when {
        vm.gitError != null -> MaterialTheme.colorScheme.error
        vm.syncNeedsAttention -> MaterialTheme.colorScheme.tertiary
        else -> MaterialTheme.colorScheme.onSurfaceVariant
    }
    TextButton(onClick = onClick) {
        Text(vm.syncLabel, color = color, maxLines = 1)
    }
}

@Composable
private fun SectionHeader(text: String) {
    Text(
        text = text.uppercase(),
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, top = 12.dp, bottom = 4.dp),
    )
}

@Composable
private fun MutedLine(text: String) {
    Text(
        text = text,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(16.dp),
    )
}

@Composable
private fun EmptyVault() {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            modifier = Modifier.padding(32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text("No notes yet", style = MaterialTheme.typography.titleMedium)
            Spacer(Modifier.height(8.dp))
            Text(
                "Tap + to create your first note.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun NoteRow(
    note: NoteSummary,
    bookmarked: Boolean,
    depth: Int = 0,
    onClick: () -> Unit,
    onToggleBookmark: () -> Unit,
    onLongClick: (() -> Unit)? = null,
) {
    ListItem(
        headlineContent = {
            Text(note.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = {
            Text(
                text = note.relative,
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        trailingContent = {
            TextButton(onClick = onToggleBookmark) {
                Text(
                    text = if (bookmarked) "★" else "☆",
                    color = if (bookmarked) {
                        MaterialTheme.colorScheme.primary
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                )
            }
        },
        modifier = Modifier
            .combinedClickable(onClick = onClick, onLongClick = onLongClick)
            .padding(start = (depth * 16).dp),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EditorScreen(vm: DragonGlassViewModel, relative: String) {
    // Back goes Preview → Edit, then Edit → list. Sheets and dialogs handle their own
    // Back before this handler sees it.
    BackHandler {
        if (vm.preview) {
            vm.togglePreview()
        } else {
            vm.closeNote()
        }
    }

    var menuOpen by remember { mutableStateOf(false) }
    var showOutline by remember { mutableStateOf(false) }
    var showDetails by remember { mutableStateOf(false) }
    var showRename by remember { mutableStateOf(false) }
    var showSync by remember { mutableStateOf(false) }
    var editorValue by remember(relative) { mutableStateOf(TextFieldValue(vm.source)) }
    val focusRequester = remember { FocusRequester() }
    val focusManager = LocalFocusManager.current
    val title = relative.substringAfterLast('/').removeSuffix(".md")

    LaunchedEffect(relative) { focusRequester.requestFocus() }

    // Select the current match so the editor scrolls it into view.
    LaunchedEffect(vm.findOpen, vm.findIndex, vm.findRanges) {
        if (!vm.findOpen) return@LaunchedEffect
        val range = vm.findRanges.getOrNull(vm.findIndex) ?: return@LaunchedEffect
        editorValue = TextFieldValue(
            text = vm.source,
            selection = TextRange(range.first, range.last + 1),
        )
        focusRequester.requestFocus()
    }

    // Jump to a full-text search hit once the note source has loaded.
    LaunchedEffect(relative, vm.pendingLine) {
        val line = vm.pendingLine ?: return@LaunchedEffect
        if (vm.source.isEmpty()) return@LaunchedEffect
        editorValue = TextFieldValue(
            text = vm.source,
            selection = TextRange(vm.offsetForLine(line)),
        )
        vm.consumePendingLine()
    }

    Scaffold(
        // Keep the caret and the status bar above the on-screen keyboard.
        modifier = Modifier.imePadding(),
        topBar = {
            TopAppBar(
                navigationIcon = {
                    TextButton(onClick = vm::closeNote) { Text("Back") }
                },
                title = {
                    Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis)
                },
                actions = {
                    TextButton(onClick = vm::togglePreview) {
                        Text(if (vm.preview) "Edit" else "Preview")
                    }
                    TextButton(onClick = { vm.save() }, enabled = vm.dirty) {
                        Text("Save")
                    }
                    SyncChip(vm) {
                        vm.clearGitMessages()
                        vm.refreshGitStatus()
                        showSync = true
                    }
                    Box {
                        TextButton(onClick = { menuOpen = true }) { Text("•••") }
                        DropdownMenu(
                            expanded = menuOpen,
                            onDismissRequest = { menuOpen = false },
                        ) {
                            DropdownMenuItem(
                                text = { Text("Find in note") },
                                onClick = {
                                    menuOpen = false
                                    vm.openFind()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Outline") },
                                onClick = {
                                    menuOpen = false
                                    showOutline = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Details") },
                                onClick = {
                                    menuOpen = false
                                    showDetails = true
                                    vm.loadTags()
                                },
                            )
                            HorizontalDivider()
                            DropdownMenuItem(
                                text = { Text("Move or rename…") },
                                onClick = {
                                    menuOpen = false
                                    showRename = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("Move to trash") },
                                onClick = {
                                    menuOpen = false
                                    vm.deleteSelected()
                                },
                            )
                        }
                    }
                },
            )
        },
        bottomBar = { StatusBar(vm) },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            if (vm.findOpen) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    OutlinedTextField(
                        value = vm.findPattern,
                        onValueChange = vm::onFindChange,
                        placeholder = { Text("Find in note") },
                        singleLine = true,
                        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                        keyboardActions = KeyboardActions(
                            onSearch = { focusManager.clearFocus() },
                        ),
                        modifier = Modifier.weight(1f),
                    )
                    Text(
                        vm.findLabel,
                        modifier = Modifier.padding(horizontal = 8.dp),
                        style = MaterialTheme.typography.labelSmall,
                    )
                    TextButton(onClick = { vm.findNext(backwards = true) }) { Text("\u2191") }
                    TextButton(onClick = { vm.findNext() }) { Text("\u2193") }
                    TextButton(onClick = vm::closeFind) { Text("x") }
                }
            }
            Box(Modifier.weight(1f).fillMaxWidth()) {
            if (vm.preview) {
                MarkdownWebView(
                    html = vm.previewHtml,
                    baseUrl = vm.baseUrl,
                    modifier = Modifier.fillMaxSize(),
                    onOpenNote = { vm.openLinkedNote(it) },
                    initialScroll = vm.previewScroll,
                    onScroll = { vm.rememberPreviewScroll(it) },
                )
            } else {
                BasicTextField(
                    value = editorValue,
                    onValueChange = {
                        editorValue = it
                        vm.onSourceChange(it.text)
                    },
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(16.dp)
                        .focusRequester(focusRequester),
                    textStyle = TextStyle(
                        fontFamily = FontFamily.Monospace,
                        fontSize = 14.sp,
                        lineHeight = 22.sp,
                        color = MaterialTheme.colorScheme.onBackground,
                    ),
                    // Markdown is punctuation, not prose: no autocorrect, no smart
                    // capitalisation, and a newline key instead of a search key.
                    keyboardOptions = KeyboardOptions(
                        capitalization = KeyboardCapitalization.None,
                        autoCorrectEnabled = false,
                        keyboardType = KeyboardType.Text,
                        imeAction = ImeAction.Default,
                    ),
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                )
            }
            }
        }
    }

    if (showSync) {
        GitSyncSheet(vm, onDismiss = { showSync = false })
    }
    if (showOutline) {
        OutlineSheet(
            vm = vm,
            onDismiss = { showOutline = false },
            onJump = { line ->
                editorValue = TextFieldValue(
                    text = vm.source,
                    selection = TextRange(vm.offsetForLine(line)),
                )
                showOutline = false
            },
        )
    }

    if (showDetails) {
        DetailsSheet(vm, onDismiss = { showDetails = false })
    }

    if (showRename) {
        TextInputDialog(
            title = "Move or rename",
            label = "Vault path",
            initial = relative,
            hint = "Add folders to move the note, e.g. notes/New name.md.",
            confirmLabel = "Move",
            onDismiss = { showRename = false },
            onConfirm = {
                vm.renameSelected(it)
                showRename = false
            },
        )
    }
}

@Composable
private fun StatusBar(vm: DragonGlassViewModel) {
    Surface(tonalElevation = 3.dp) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            val stats = vm.stats
            Text(
                text = "${stats?.words ?: 0} words · ${stats?.readingMinutes ?: 0} min",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                text = if (vm.dirty) "Unsaved" else "Saved",
                style = MaterialTheme.typography.labelSmall,
                color = if (vm.dirty) {
                    MaterialTheme.colorScheme.error
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun OutlineSheet(
    vm: DragonGlassViewModel,
    onDismiss: () -> Unit,
    onJump: (line: Int) -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "Outline",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            if (vm.headings.isEmpty()) {
                Text(
                    text = "No headings yet.",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                LazyColumn(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 480.dp),
                ) {
                    items(vm.headings) { heading ->
                        OutlineRow(heading) { onJump(heading.line) }
                    }
                }
            }
        }
    }
}

@Composable
private fun OutlineRow(heading: HeadingInfo, onClick: () -> Unit) {
    ListItem(
        headlineContent = {
            Text(
                text = heading.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        },
        supportingContent = {
            Text(
                text = "H${heading.level} · line ${heading.line}",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        modifier = Modifier
            .clickable(onClick = onClick)
            .padding(start = ((heading.level - 1).coerceAtLeast(0) * 12).dp),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun DetailsSheet(vm: DragonGlassViewModel, onDismiss: () -> Unit) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 560.dp)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "Details",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )

            val stats = vm.stats
            if (stats != null) {
                Text(
                    text = "${stats.words} words · ${stats.characters} chars · " +
                        "${stats.paragraphs} paragraphs · ${stats.readingMinutes} min",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            SectionTitle("Properties")
            if (vm.properties.isEmpty()) {
                Muted("No frontmatter properties.")
            } else {
                vm.properties.forEach { property ->
                    Row(Modifier.fillMaxWidth().padding(vertical = 3.dp)) {
                        Text(
                            text = property.key,
                            modifier = Modifier.width(120.dp),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        Text(text = property.value)
                    }
                }
            }

            SectionTitle("Relations")
            if (vm.relations.isEmpty()) {
                Muted("No links or relations.")
            } else {
                vm.relations.forEach { relation ->
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(vertical = 4.dp),
                    ) {
                        Text(
                            text = relation.kind,
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.primary,
                        )
                        Spacer(Modifier.width(8.dp))
                        Text(
                            text = (if (relation.incoming) "← " else "→ ") + relation.target,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }

            SectionTitle("Tags")
            if (vm.tags.isEmpty()) {
                Muted("No tags in this vault.")
            } else {
                vm.tags.forEach { tag ->
                    Text(
                        text = "#${tag.tag}  (${tag.notes})",
                        modifier = Modifier.padding(vertical = 2.dp),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(
        text = text,
        style = MaterialTheme.typography.titleSmall,
        modifier = Modifier.padding(top = 20.dp, bottom = 6.dp),
    )
}

@Composable
private fun Muted(text: String) {
    Text(
        text = text,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        style = MaterialTheme.typography.bodySmall,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun GitSyncSheet(vm: DragonGlassViewModel, onDismiss: () -> Unit) {
    var remote by remember { mutableStateOf(vm.gitRemote) }
    var branch by remember { mutableStateOf(vm.gitBranch) }
    var token by remember { mutableStateOf("") }
    var confirmClone by remember { mutableStateOf(false) }
    val status = vm.gitStatus

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(max = 640.dp)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "Git sync",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            Text(
                text = statusLine(status),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodySmall,
            )

            OutlinedTextField(
                value = remote,
                onValueChange = { remote = it },
                label = { Text("GitHub remote") },
                placeholder = { Text("https://github.com/owner/vault.git") },
                singleLine = true,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 16.dp),
            )
            OutlinedTextField(
                value = branch,
                onValueChange = { branch = it },
                label = { Text("Branch") },
                singleLine = true,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 12.dp),
            )
            OutlinedTextField(
                value = token,
                onValueChange = { token = it },
                label = { Text("Personal access token") },
                placeholder = {
                    Text(if (vm.gitHasToken) "Stored — leave blank to keep" else "github_pat_…")
                },
                singleLine = true,
                visualTransformation = PasswordVisualTransformation(),
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 12.dp),
            )
            Text(
                text = "Use a fine-grained token with Contents: Read and write. " +
                    "The token is encrypted with Android Keystore and never written to the vault.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 6.dp),
            )
            if (vm.gitHasToken) {
                TextButton(onClick = vm::clearGitToken) { Text("Forget stored token") }
            }

            if (vm.gitBusy) {
                LinearProgressIndicator(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 16.dp),
                )
            }

            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Button(
                    onClick = {
                        vm.saveGitSettings(remote, branch, token)
                        token = ""
                    },
                    enabled = !vm.gitBusy,
                ) { Text("Save") }

                Button(
                    onClick = {
                        vm.saveGitSettings(remote, branch, token)
                        token = ""
                        vm.syncGit()
                    },
                    enabled = !vm.gitBusy && status?.isRepo == true,
                ) { Text("Sync") }
            }

            if (status?.isRepo != true) {
                Text(
                    text = "No repository in this vault yet. Link the GitHub repo to replace " +
                        "local notes with its contents, or create a new repository from this vault.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 16.dp),
                )
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    OutlinedButton(
                        onClick = { confirmClone = true },
                        enabled = !vm.gitBusy && remote.isNotBlank(),
                    ) { Text("Link existing") }
                    Button(
                        onClick = {
                            vm.saveGitSettings(remote, branch, token)
                            token = ""
                            vm.initGitRepository()
                        },
                        enabled = !vm.gitBusy && remote.isNotBlank(),
                    ) { Text("Create new") }
                }
            }

            vm.gitMessage?.let {
                Text(
                    text = it,
                    color = MaterialTheme.colorScheme.primary,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 12.dp),
                )
            }
            vm.gitError?.let {
                Text(
                    text = it,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodySmall,
                    modifier = Modifier.padding(top = 12.dp),
                )
            }
            if (vm.gitConflicts.isNotEmpty()) {
                Text(
                    text = "Conflicted files",
                    style = MaterialTheme.typography.titleSmall,
                    modifier = Modifier.padding(top = 12.dp),
                )
                vm.gitConflicts.forEach { file ->
                    Text(
                        text = file,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
                OutlinedButton(
                    onClick = vm::abortGitRebase,
                    enabled = !vm.gitBusy,
                    modifier = Modifier.padding(top = 8.dp),
                ) { Text("Abort sync") }
            }
        }
    }

    if (confirmClone) {
        AlertDialog(
            onDismissRequest = { confirmClone = false },
            title = { Text("Link existing repository?") },
            text = {
                Text(
                    "The local vault will be replaced by the GitHub repository. " +
                        "A backup of the current vault is kept on the device.",
                )
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmClone = false
                        vm.saveGitSettings(remote, branch, token)
                        token = ""
                        vm.cloneGitRepository()
                    },
                ) { Text("Replace vault") }
            },
            dismissButton = {
                TextButton(onClick = { confirmClone = false }) { Text("Cancel") }
            },
        )
    }
}

private fun statusLine(status: GitStatus?): String {
    if (status == null) return "Checking repository…"
    if (!status.isRepo) return "Not a git repository yet."
    return buildString {
        append(status.branch.ifBlank { "main" })
        if (!status.hasRemote) append(" · no remote")
        append(" · ")
        if (status.hasLocalWork) {
            append("${status.changed} changed")
            if (status.untracked > 0) append(", ${status.untracked} untracked")
        } else {
            append("clean")
        }
        if (status.ahead > 0) append(" · ↑${status.ahead}")
        if (status.behind > 0) append(" · ↓${status.behind}")
        if (status.conflicts.isNotEmpty()) append(" · ${status.conflicts.size} conflicts")
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun GraphSheet(vm: DragonGlassViewModel, onDismiss: () -> Unit) {
    val graph = vm.graph
    val layout = remember(graph) { graph?.let(::layoutGraph).orEmpty() }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "Graph",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            when {
                vm.graphLoading || graph == null -> {
                    if (vm.graphLoading) {
                        LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    } else {
                        Muted("No notes to graph yet.")
                    }
                }
                graph.nodes.isEmpty() -> Muted("No notes to graph yet.")
                else -> {
                    Text(
                        text = "${graph.nodes.size} notes · ${graph.edges.size} links",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                    )
                    GraphCanvas(
                        graph = graph,
                        layout = layout,
                        modifier = Modifier
                            .fillMaxWidth()
                            .height(420.dp)
                            .padding(top = 8.dp),
                        onOpen = { relative ->
                            onDismiss()
                            vm.openNote(relative)
                        },
                    )
                    Text(
                        text = "Tap a node to open its note. Node size follows the link count.",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun GraphCanvas(
    graph: GraphView,
    layout: Map<Int, Offset>,
    modifier: Modifier = Modifier,
    onOpen: (String) -> Unit,
) {
    val nodeColor = MaterialTheme.colorScheme.primary
    val edgeColor = MaterialTheme.colorScheme.outline
    val maxLinks = graph.nodes.maxOfOrNull { it.links }?.coerceAtLeast(1) ?: 1

    Canvas(
        modifier = modifier.pointerInput(graph, layout) {
            detectTapGestures { tap ->
                val width = size.width.toFloat()
                val height = size.height.toFloat()
                if (width <= 0f || height <= 0f) return@detectTapGestures
                var nearest: Int? = null
                var nearestDistance = Float.MAX_VALUE
                for ((id, position) in layout) {
                    val dx = position.x * width - tap.x
                    val dy = position.y * height - tap.y
                    val distance = sqrt(dx * dx + dy * dy)
                    if (distance < nearestDistance) {
                        nearest = id
                        nearestDistance = distance
                    }
                }
                val id = nearest ?: return@detectTapGestures
                if (nearestDistance > 48f) return@detectTapGestures
                graph.nodes.firstOrNull { it.id == id }?.let { onOpen(it.relative) }
            }
        },
    ) {
        val width = size.width
        val height = size.height
        fun center(id: Int): Offset? = layout[id]?.let { Offset(it.x * width, it.y * height) }

        for (edge in graph.edges) {
            val from = center(edge.from) ?: continue
            val to = center(edge.to) ?: continue
            drawLine(
                color = edgeColor.copy(alpha = 0.35f),
                start = from,
                end = to,
                strokeWidth = 1.5f,
            )
        }

        for (node in graph.nodes) {
            val point = center(node.id) ?: continue
            val weight = sqrt(node.links.toFloat() / maxLinks)
            drawCircle(
                color = nodeColor.copy(alpha = 0.85f),
                radius = 4f + 10f * weight,
                center = point,
            )
        }
    }
}

/// Positions nodes in unit space with a small force-directed pass, falling back to a circle
/// for very large vaults so the layout stays instant.
private fun layoutGraph(graph: GraphView): Map<Int, Offset> {
    val nodes = graph.nodes
    val count = nodes.size
    if (count == 0) return emptyMap()
    if (count == 1) return mapOf(nodes[0].id to Offset(0.5f, 0.5f))

    val xs = FloatArray(count)
    val ys = FloatArray(count)
    for (i in 0 until count) {
        val angle = i.toFloat() / count * 2f * PI.toFloat()
        xs[i] = 0.5f + 0.36f * cos(angle)
        ys[i] = 0.5f + 0.36f * sin(angle)
    }

    val iterations = when {
        count > 400 -> 0
        count > 160 -> 120
        else -> 260
    }
    if (iterations > 0) {
        val index = HashMap<Int, Int>(count)
        nodes.forEachIndexed { i, node -> index[node.id] = i }
        val links = ArrayList<IntArray>(graph.edges.size)
        for (edge in graph.edges) {
            val from = index[edge.from] ?: continue
            val to = index[edge.to] ?: continue
            if (from != to) links.add(intArrayOf(from, to))
        }

        val k = 1.1f / sqrt(count.toFloat())
        val dispX = FloatArray(count)
        val dispY = FloatArray(count)

        repeat(iterations) { step ->
            java.util.Arrays.fill(dispX, 0f)
            java.util.Arrays.fill(dispY, 0f)

            for (i in 0 until count) {
                for (j in i + 1 until count) {
                    var dx = xs[i] - xs[j]
                    var dy = ys[i] - ys[j]
                    var distanceSquared = dx * dx + dy * dy
                    if (distanceSquared < 1e-6f) {
                        dx = 0.001f * (i + 1)
                        dy = 0.001f * (j + 1)
                        distanceSquared = dx * dx + dy * dy
                    }
                    val distance = sqrt(distanceSquared)
                    val force = k * k / distance
                    val fx = dx / distance * force
                    val fy = dy / distance * force
                    dispX[i] += fx
                    dispY[i] += fy
                    dispX[j] -= fx
                    dispY[j] -= fy
                }
            }

            for (link in links) {
                val from = link[0]
                val to = link[1]
                val dx = xs[from] - xs[to]
                val dy = ys[from] - ys[to]
                val distance = sqrt(dx * dx + dy * dy).coerceAtLeast(1e-4f)
                val force = distance * distance / k
                val fx = dx / distance * force
                val fy = dy / distance * force
                dispX[from] -= fx
                dispY[from] -= fy
                dispX[to] += fx
                dispY[to] += fy
            }

            val temperature = 0.09f * (1f - step / iterations.toFloat()) + 0.004f
            for (i in 0 until count) {
                val length = sqrt(dispX[i] * dispX[i] + dispY[i] * dispY[i])
                    .coerceAtLeast(1e-6f)
                val scale = min(length, temperature) / length
                xs[i] = (xs[i] + dispX[i] * scale).coerceIn(0.03f, 0.97f)
                ys[i] = (ys[i] + dispY[i] * scale).coerceIn(0.03f, 0.97f)
            }
        }
    }

    val positions = HashMap<Int, Offset>(count)
    nodes.forEachIndexed { i, node -> positions[node.id] = Offset(xs[i], ys[i]) }
    return positions
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TemplatesSheet(
    vm: DragonGlassViewModel,
    onDismiss: () -> Unit,
    onCreated: () -> Unit,
) {
    var title by remember { mutableStateOf("") }

    LaunchedEffect(Unit) { vm.loadTemplates() }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "New from template",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            OutlinedTextField(
                value = title,
                onValueChange = { title = it },
                label = { Text("Note title") },
                placeholder = { Text("Leave blank to use the template name") },
                singleLine = true,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 8.dp),
            )
            if (vm.templates.isEmpty()) {
                Muted(
                    "No templates yet. Add markdown files to the vault's Templates/ folder " +
                        "to use them here.",
                )
            } else {
                LazyColumn(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(max = 360.dp)
                        .padding(top = 8.dp),
                ) {
                    items(vm.templates, key = { it.relative }) { template ->
                        ListItem(
                            headlineContent = { Text(template.name) },
                            supportingContent = {
                                Text(
                                    text = template.relative,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            },
                            modifier = Modifier.clickable {
                                vm.createFromTemplate(
                                    template.relative,
                                    title.ifBlank { template.name },
                                )
                                onCreated()
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun TextInputDialog(
    title: String,
    label: String,
    initial: String,
    hint: String,
    confirmLabel: String,
    onDismiss: () -> Unit,
    onConfirm: (String) -> Unit,
) {
    var value by remember { mutableStateOf(initial) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column {
                OutlinedTextField(
                    value = value,
                    onValueChange = { value = it },
                    label = { Text(label) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
                Text(
                    text = hint,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(top = 8.dp),
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onConfirm(value) }) { Text(confirmLabel) }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun VaultsSheet(
    vm: DragonGlassViewModel,
    onDismiss: () -> Unit,
    onImport: () -> Unit,
    onExport: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "Vaults",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            Muted(
                "A vault imported from device storage stays linked to that folder: saves are " +
                    "written back, and Sync pulls edits made elsewhere. Import copies a new " +
                    "folder in; Export writes the current notes to a folder you pick.",
            )
            LazyColumn(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 360.dp)
                    .padding(top = 8.dp),
            ) {
                items(vm.vaults, key = { it.id }) { vault ->
                    StoredVaultRow(
                        vault = vault,
                        active = vault.id == vm.activeVaultId,
                        onOpen = {
                            vm.switchVault(vault)
                            onDismiss()
                        },
                    )
                }
            }
            if (vm.canSyncWithFolder) {
                Button(
                    onClick = {
                        onDismiss()
                        vm.syncVault()
                    },
                    modifier = Modifier.padding(top = 12.dp),
                ) { Text("Sync with folder") }
            }
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(top = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedButton(onClick = onImport) { Text("Import folder…") }
                OutlinedButton(onClick = onExport) { Text("Export vault…") }
            }
        }
    }
}

@Composable
private fun StoredVaultRow(vault: StoredVault, active: Boolean, onOpen: () -> Unit) {
    ListItem(
        headlineContent = {
            Text(vault.name, maxLines = 1, overflow = TextOverflow.Ellipsis)
        },
        supportingContent = {
            Text(
                text = if (vault.id == "default") {
                    "App storage"
                } else {
                    "Imported from device storage"
                },
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        },
        trailingContent = {
            if (active) {
                Text(
                    text = "Current",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.primary,
                )
            } else {
                TextButton(onClick = onOpen) { Text("Open") }
            }
        },
        modifier = Modifier.clickable(enabled = !active, onClick = onOpen),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun TrashSheet(vm: DragonGlassViewModel, onDismiss: () -> Unit) {
    var confirmEmpty by remember { mutableStateOf(false) }

    LaunchedEffect(Unit) { vm.loadTrash() }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp)
                .padding(bottom = 24.dp),
        ) {
            Text(
                text = "Trash",
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.padding(vertical = 8.dp),
            )
            Muted("Deleted notes stay here until you empty the trash.")
            when {
                vm.trashLoading -> LinearProgressIndicator(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(top = 16.dp),
                )
                vm.trash.isEmpty() -> Muted("The trash is empty.")
                else -> {
                    LazyColumn(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 420.dp)
                            .padding(top = 8.dp),
                    ) {
                        items(vm.trash, key = { it.trashRelative }) { entry ->
                            ListItem(
                                headlineContent = {
                                    Text(
                                        text = entry.original,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                },
                                supportingContent = {
                                    Text(
                                        text = "deleted ${trashTime(entry.deletedAt)}",
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                },
                                trailingContent = {
                                    TextButton(
                                        onClick = { vm.restoreTrashEntry(entry.trashRelative) },
                                    ) { Text("Restore") }
                                },
                            )
                        }
                    }
                    OutlinedButton(
                        onClick = { confirmEmpty = true },
                        modifier = Modifier.padding(top = 12.dp),
                    ) { Text("Empty trash") }
                }
            }
        }
    }

    if (confirmEmpty) {
        AlertDialog(
            onDismissRequest = { confirmEmpty = false },
            title = { Text("Empty the trash?") },
            text = { Text("Every deleted note in the trash will be removed permanently.") },
            confirmButton = {
                TextButton(
                    onClick = {
                        confirmEmpty = false
                        vm.emptyTrash()
                    },
                ) { Text("Empty") }
            },
            dismissButton = {
                TextButton(onClick = { confirmEmpty = false }) { Text("Cancel") }
            },
        )
    }
}

private fun trashTime(millis: Long): String {
    if (millis <= 0L) return "at an unknown time"
    val formatter = java.time.format.DateTimeFormatter
        .ofPattern("MMM d, HH:mm")
        .withZone(java.time.ZoneId.systemDefault())
    return formatter.format(java.time.Instant.ofEpochMilli(millis))
}

@Composable
private fun MarkdownWebView(
    html: String,
    baseUrl: String,
    modifier: Modifier = Modifier,
    onOpenNote: (String) -> Unit = {},
    initialScroll: Int = 0,
    onScroll: (Int) -> Unit = {},
) {
    val dark = isSystemInDarkTheme()
    // Comfortable defaults for a phone or tablet, plus the dark palette when needed.
    val themed = remember(html, dark) {
        val base = html.replace("</head>", "<style>$PREVIEW_CSS</style></head>")
        if (dark) {
            base.replace("</head>", "<style>$DARK_PREVIEW_CSS</style></head>")
        } else {
            base
        }
    }

    AndroidView(
        modifier = modifier,
        factory = { viewContext ->
            WebView(viewContext).apply {
                settings.javaScriptEnabled = false
                settings.allowFileAccess = true
                settings.allowContentAccess = true
                settings.loadWithOverviewMode = true
                settings.useWideViewPort = true
                // Pinch to zoom without the floating +/- buttons.
                settings.builtInZoomControls = true
                settings.displayZoomControls = false
                setBackgroundColor(android.graphics.Color.TRANSPARENT)
                webViewClient = object : WebViewClient() {
                    override fun onPageFinished(view: WebView?, url: String?) {
                        // Restore the previous preview position after a reload.
                        view?.post { view.scrollTo(0, initialScroll) }
                    }

                    override fun shouldOverrideUrlLoading(
                        view: WebView?,
                        request: WebResourceRequest?,
                    ): Boolean {
                        val url = request?.url?.toString() ?: return false
                        val handled = handlePreviewLink(
                            url = url,
                            baseUrl = baseUrl,
                            onOpenNote = onOpenNote,
                            openExternally = { external ->
                                runCatching {
                                    viewContext.startActivity(
                                        Intent(Intent.ACTION_VIEW, Uri.parse(external)),
                                    )
                                }
                            },
                        )
                        return handled
                    }
                }
            }
        },
        update = { webView ->
            // Reloading on every recomposition used to reset the scroll and re-lay out the
            // page; only load when the rendered document actually changed.
            if (webView.tag != themed) {
                webView.tag = themed
                webView.loadDataWithBaseURL(baseUrl, themed, "text/html", "utf-8", null)
            }
        },
        onRelease = { webView ->
            onScroll(webView.scrollY)
        },
    )
}

/// Route a link tapped in the preview.
///
/// Internal `.html` pages map back to their `.md` note and open it in the app; external
/// links leave for the system browser; anything else is blocked so the preview document
/// is never replaced in place. Returns whether the navigation was handled.
private fun handlePreviewLink(
    url: String,
    baseUrl: String,
    onOpenNote: (String) -> Unit,
    openExternally: (String) -> Unit,
): Boolean {
    if (url == "about:blank" || url.startsWith("data:")) {
        return false
    }

    if (url.startsWith(baseUrl)) {
        val relative = url
            .removePrefix(baseUrl)
            .substringBefore('#')
            .substringBefore('?')
        if (relative.isBlank()) {
            // A same-page anchor: let the WebView scroll to it.
            return false
        }
        val note = when {
            relative.endsWith(".html", ignoreCase = true) ->
                relative.dropLast(5) + ".md"
            relative.endsWith(".htm", ignoreCase = true) ->
                relative.dropLast(4) + ".md"
            relative.endsWith(".md", ignoreCase = true) -> relative
            // Attachments: leave the preview document alone.
            else -> return true
        }
        onOpenNote(Uri.decode(note))
        return true
    }

    // A file outside the vault must not replace the preview document.
    if (url.startsWith("file:")) {
        return true
    }

    if (url.startsWith("http://") || url.startsWith("https://") || url.startsWith("mailto:")) {
        openExternally(url)
        return true
    }

    // Unknown scheme or off-vault target: never navigate the preview away from the note.
    return true
}

/// Reading defaults for the preview: a comfortable measure and type size on phones and
/// tablets, and images that never overflow the page.
private const val PREVIEW_CSS = """
body{max-width:44rem;margin:0 auto;padding:16px;font-size:16px;line-height:1.65;
  -webkit-text-size-adjust:100%}
img{max-width:100%;height:auto}
pre,table{max-width:100%;overflow-x:auto}
"""

/// The engine's export CSS targets light pages, so dark mode overrides the few colors that
/// would otherwise be dark-on-dark. Math is drawn as black SVG, hence the invert.
private const val DARK_PREVIEW_CSS = """
body{color:#E4E7EE}
h1,h2,h3,h4,h5,h6{color:#F2F4F8}
a,.fn{color:#9DB8FF}
blockquote{border-left-color:#3A4150;color:#B9C0CF}
hr{border-top-color:#3A4150}
mark{background:#4A4420;color:#E4E7EE}
pre,code,th,.math{background:#222733}
th,td{border-color:#3A4150}
.footnote{color:#B9C0CF}
.math svg,.math-display svg{filter:invert(1)}
"""
