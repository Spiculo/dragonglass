package com.dragonglass.app.ui

import android.webkit.WebView
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
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
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import com.dragonglass.app.DragonGlassViewModel
import com.dragonglass.app.sync.GitStatus
import uniffi.dragonglass.HeadingInfo
import uniffi.dragonglass.NoteSummary

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

    Box(Modifier.fillMaxSize()) {
        val selected = vm.selected
        if (selected == null) {
            NotesScreen(vm)
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

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun NotesScreen(vm: DragonGlassViewModel) {
    var showSync by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text("Dragon Glass", fontWeight = FontWeight.SemiBold)
                        Text(
                            text = vm.vaultLabel,
                            style = MaterialTheme.typography.labelSmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                },
                actions = {
                    TextButton(
                        onClick = {
                            vm.clearGitMessages()
                            vm.refreshGitStatus()
                            showSync = true
                        },
                    ) { Text("Sync") }
                    TextButton(onClick = vm::openDaily) { Text("Today") }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(onClick = vm::newNote) {
                Text("+", fontSize = 26.sp)
            }
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            OutlinedTextField(
                value = vm.query,
                onValueChange = vm::onQueryChange,
                placeholder = { Text("Search notes") },
                singleLine = true,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 8.dp),
            )

            when {
                vm.notes.isEmpty() && !vm.loading -> EmptyVault()
                vm.visibleNotes.isEmpty() -> Text(
                    text = "No notes match “${vm.query}”.",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(16.dp),
                )
                else -> LazyColumn(Modifier.fillMaxSize()) {
                    items(vm.visibleNotes, key = { it.relative }) { note ->
                        NoteRow(
                            note = note,
                            bookmarked = note.relative in vm.bookmarks,
                            onClick = { vm.openNote(note.relative) },
                            onToggleBookmark = { vm.toggleBookmark(note.relative) },
                        )
                        HorizontalDivider(color = MaterialTheme.colorScheme.outline.copy(alpha = 0.25f))
                    }
                }
            }
        }
    }

    if (showSync) {
        GitSyncSheet(vm, onDismiss = { showSync = false })
    }
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

@Composable
private fun NoteRow(
    note: NoteSummary,
    bookmarked: Boolean,
    onClick: () -> Unit,
    onToggleBookmark: () -> Unit,
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
        modifier = Modifier.clickable(onClick = onClick),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EditorScreen(vm: DragonGlassViewModel, relative: String) {
    BackHandler { vm.closeNote() }

    var menuOpen by remember { mutableStateOf(false) }
    var showOutline by remember { mutableStateOf(false) }
    var showDetails by remember { mutableStateOf(false) }
    var editorValue by remember(relative) { mutableStateOf(TextFieldValue(vm.source)) }
    val focusRequester = remember { FocusRequester() }
    val title = relative.substringAfterLast('/').removeSuffix(".md")

    LaunchedEffect(relative) { focusRequester.requestFocus() }

    Scaffold(
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
                    Box {
                        TextButton(onClick = { menuOpen = true }) { Text("•••") }
                        DropdownMenu(
                            expanded = menuOpen,
                            onDismissRequest = { menuOpen = false },
                        ) {
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
                                text = { Text("Delete note") },
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
        Box(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            if (vm.preview) {
                MarkdownWebView(
                    html = vm.previewHtml,
                    baseUrl = vm.baseUrl,
                    modifier = Modifier.fillMaxSize(),
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
                    cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                )
            }
        }
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

@Composable
private fun MarkdownWebView(
    html: String,
    baseUrl: String,
    modifier: Modifier = Modifier,
) {
    AndroidView(
        modifier = modifier,
        factory = { context ->
            WebView(context).apply {
                settings.javaScriptEnabled = false
                settings.allowFileAccess = true
                settings.allowContentAccess = true
                settings.loadWithOverviewMode = true
                settings.useWideViewPort = true
                setBackgroundColor(android.graphics.Color.TRANSPARENT)
            }
        },
        update = { webView ->
            webView.loadDataWithBaseURL(baseUrl, html, "text/html", "utf-8", null)
        },
    )
}
