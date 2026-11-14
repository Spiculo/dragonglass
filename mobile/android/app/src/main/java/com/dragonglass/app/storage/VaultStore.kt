package com.dragonglass.app.storage

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.documentfile.provider.DocumentFile
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

data class StoredVault(
    val id: String,
    val name: String,
    val root: File,
    /** The SAF tree this vault was imported from or last exported to, when known. */
    val treeUri: String?,
)

/**
 * App-private vaults plus the SAF folders they came from.
 *
 * The engine is `std::fs` based, so every vault lives in app-private storage. Importing
 * copies a folder in, and the copy stays linked to its `content://` tree: saves are written
 * back, structural changes mirror the tree, and opening the vault pulls newer documents.
 * The app-private copy remains the source of truth, which keeps the shared engine unchanged.
 */
class VaultStore(private val context: Context) {
    /// Thrown when the user cancels a long import or export.
    class TransferCancelled : Exception("Transfer cancelled")

    private val preferences =
        context.getSharedPreferences("dragon_glass_vaults", Context.MODE_PRIVATE)
    private val vaultsDirectory = File(context.filesDir, "vaults")

    val defaultRoot: File = File(context.filesDir, "vault")

    val activeId: String?
        get() = preferences.getString("active", null)

    fun activeVault(): StoredVault {
        val id = activeId ?: return defaultVault()
        return find(id) ?: defaultVault()
    }

    fun isDefault(vault: StoredVault): Boolean = vault.id == DEFAULT_ID

    /** All vaults the app knows about, the default one first. */
    fun vaults(): List<StoredVault> {
        val stored = readMeta().mapNotNull { meta ->
            val root = File(vaultsDirectory, meta.id)
            if (!root.isDirectory) {
                null
            } else {
                StoredVault(meta.id, meta.name, root, meta.treeUri)
            }
        }
        return listOf(defaultVault()) + stored
    }

    fun activate(id: String?) {
        if (id == null || id == DEFAULT_ID) {
            preferences.edit().remove("active").apply()
        } else {
            preferences.edit().putString("active", id).apply()
        }
    }

    /** Copy a SAF tree into a fresh app-private vault and make it the active one. */
    fun importTree(
        treeUri: Uri,
        onProgress: (Int) -> Unit = {},
        isCancelled: () -> Boolean = { false },
    ): StoredVault {
        takePermission(treeUri, write = false)

        val source = DocumentFile.fromTreeUri(context, treeUri)
            ?: throw IllegalStateException("The selected folder is not readable")
        val name = source.name?.takeIf { it.isNotBlank() } ?: "Imported vault"
        val id = UUID.randomUUID().toString().replace("-", "").take(10)
        val root = File(vaultsDirectory, id)
        if (!root.mkdirs()) {
            throw IllegalStateException("Could not create local storage for $name")
        }

        val copied = try {
            copyInto(source, root, onProgress, isCancelled)
        } catch (error: Throwable) {
            // A cancelled or failed import leaves no half-vault behind.
            root.deleteRecursively()
            throw error
        }
        if (copied == 0) {
            root.deleteRecursively()
            throw IllegalStateException("That folder has no markdown or attachment files to import")
        }

        val metas = readMeta()
        metas.add(Meta(id, name, treeUri.toString()))
        writeMeta(metas)
        activate(id)
        return StoredVault(id, name, root, treeUri.toString())
    }

    /** Write the active vault into a SAF tree, merging by name. Returns the file count. */
    fun exportTree(
        vault: StoredVault,
        treeUri: Uri,
        onProgress: (Int) -> Unit = {},
        isCancelled: () -> Boolean = { false },
    ): Int {
        takePermission(treeUri, write = true)

        val target = DocumentFile.fromTreeUri(context, treeUri)
            ?: throw IllegalStateException("The selected folder is not writable")
        val written = copyOut(vault.root, target, onProgress, isCancelled)
        if (written == 0) {
            throw IllegalStateException("There are no notes to export yet")
        }

        val metas = readMeta()
        val existing = metas.indexOfFirst { it.id == vault.id }
        if (existing >= 0) {
            metas[existing] = metas[existing].copy(treeUri = treeUri.toString())
        } else {
            metas.add(Meta(vault.id, vault.name, treeUri.toString()))
        }
        writeMeta(metas)
        return written
    }

    private fun copyInto(
        source: DocumentFile,
        target: File,
        onProgress: (Int) -> Unit,
        isCancelled: () -> Boolean,
    ): Int {
        var copied = 0
        for (child in source.listFiles()) {
            if (isCancelled()) throw TransferCancelled()
            val name = child.name ?: continue
            // Dot folders are Obsidian/git metadata; the app has its own.
            if (name.startsWith(".")) continue
            if (child.isDirectory) {
                val directory = File(target, name)
                directory.mkdirs()
                copied += copyInto(child, directory, onProgress, isCancelled)
            } else if (child.isFile) {
                val output = File(target, name)
                val input = context.contentResolver.openInputStream(child.uri) ?: continue
                input.use { stream ->
                    output.outputStream().use { file -> stream.copyTo(file) }
                }
                copied++
                onProgress(copied)
            }
        }
        return copied
    }

    private fun copyOut(
        source: File,
        target: DocumentFile,
        onProgress: (Int) -> Unit,
        isCancelled: () -> Boolean,
    ): Int {
        var written = 0
        for (file in source.listFiles().orEmpty()) {
            if (isCancelled()) throw TransferCancelled()
            if (file.name.startsWith(".")) continue
            if (file.isDirectory) {
                val directory = target.findFile(file.name)?.takeIf { it.isDirectory }
                    ?: target.createDirectory(file.name)
                    ?: continue
                written += copyOut(file, directory, onProgress, isCancelled)
            } else {
                val existing = target.findFile(file.name)?.takeIf { it.isFile }
                val document = existing
                    ?: target.createFile("application/octet-stream", file.name)
                    ?: continue
                val output = context.contentResolver.openOutputStream(document.uri, "wt")
                    ?: continue
                file.inputStream().use { input ->
                    output.use { stream -> input.copyTo(stream) }
                }
                written++
                onProgress(written)
            }
        }
        return written
    }

    /** Copy documents from the folder that are newer (or new) into the local vault. */
    fun pullNewer(vault: StoredVault): Int {
        val tree = treeFor(vault) ?: return 0
        return pullDirectory(tree, vault.root, "")
    }

    /** Write one note back to the folder after a save. */
    fun pushFile(vault: StoredVault, relative: String): Boolean {
        val tree = treeFor(vault) ?: return false
        val local = File(vault.root, relative)
        if (!local.isFile) return false

        val parent = remoteDirectory(tree, relative.substringBeforeLast('/', "")) ?: return false
        val existing = parent.findFile(local.name)?.takeIf { it.isFile }
        val document = existing
            ?: parent.createFile("application/octet-stream", local.name)
            ?: return false
        val output = context.contentResolver.openOutputStream(document.uri, "wt") ?: return false
        local.inputStream().use { input ->
            output.use { stream -> input.copyTo(stream) }
        }
        return true
    }

    /** Mirror the whole local vault into the folder and prune files deleted locally. */
    fun pushAll(vault: StoredVault): Int {
        val tree = treeFor(vault) ?: return 0
        val written = copyOut(vault.root, tree, {}, { false })
        pruneRemote(tree, vault.root)
        return written
    }

    /** Pull newer documents, then mirror local changes, destructively. */
    fun sync(vault: StoredVault): Pair<Int, Int> {
        val tree = treeFor(vault) ?: return 0 to 0
        val pulled = pullDirectory(tree, vault.root, "")
        val written = copyOut(vault.root, tree, {}, { false })
        pruneRemote(tree, vault.root)
        return pulled to written
    }

    private fun pullDirectory(source: DocumentFile, target: File, prefix: String): Int {
        var pulled = 0
        for (child in source.listFiles()) {
            val name = child.name ?: continue
            if (name.startsWith(".")) continue
            val relative = if (prefix.isEmpty()) name else "$prefix/$name"
            if (child.isDirectory) {
                val directory = File(target, name)
                directory.mkdirs()
                pulled += pullDirectory(child, directory, relative)
            } else if (child.isFile) {
                val local = File(target, name)
                val remoteTime = child.lastModified()
                val newer = remoteTime > 0 && remoteTime > local.lastModified() + 1_000
                if (!local.exists() || newer) {
                    if (copyIn(child, local)) pulled++
                }
            }
        }
        return pulled
    }

    private fun copyIn(document: DocumentFile, target: File): Boolean {
        val input = context.contentResolver.openInputStream(document.uri) ?: return false
        try {
            input.use { stream ->
                target.outputStream().use { file -> stream.copyTo(file) }
            }
            val modified = document.lastModified()
            if (modified > 0) target.setLastModified(modified)
            return true
        } catch (_: java.io.IOException) {
            return false
        }
    }

    private fun remoteDirectory(tree: DocumentFile, path: String): DocumentFile? {
        var current = tree
        if (path.isBlank()) return current
        for (segment in path.split('/')) {
            if (segment.isEmpty()) continue
            current = current.findFile(segment)?.takeIf { it.isDirectory }
                ?: current.createDirectory(segment)
                ?: return null
        }
        return current
    }

    private fun pruneRemote(directory: DocumentFile, local: File) {
        for (child in directory.listFiles()) {
            val name = child.name ?: continue
            if (name.startsWith(".")) continue
            val localChild = File(local, name)
            if (!localChild.exists()) {
                child.delete()
            } else if (child.isDirectory) {
                pruneRemote(child, localChild)
            }
        }
    }

    private fun treeFor(vault: StoredVault): DocumentFile? {
        val uri = vault.treeUri ?: return null
        val parsed = Uri.parse(uri)
        takePermission(parsed, write = true)
        return DocumentFile.fromTreeUri(context, parsed)
    }

    private fun takePermission(treeUri: Uri, write: Boolean) {
        val flags = Intent.FLAG_GRANT_READ_URI_PERMISSION or
            (if (write) Intent.FLAG_GRANT_WRITE_URI_PERMISSION else 0)
        try {
            context.contentResolver.takePersistableUriPermission(treeUri, flags)
        } catch (_: SecurityException) {
            // Some providers hand out non-persistable grants; the copy still works.
        }
    }

    private fun defaultVault(): StoredVault =
        StoredVault(DEFAULT_ID, "On this device", defaultRoot, null)

    private fun find(id: String): StoredVault? {
        val meta = readMeta().firstOrNull { it.id == id } ?: return null
        val root = File(vaultsDirectory, meta.id)
        return if (root.isDirectory) StoredVault(meta.id, meta.name, root, meta.treeUri) else null
    }

    private fun readMeta(): MutableList<Meta> {
        val raw = preferences.getString("vaults", "[]") ?: "[]"
        return runCatching {
            val array = JSONArray(raw)
            MutableList(array.length()) { index ->
                val item = array.getJSONObject(index)
                Meta(
                    id = item.getString("id"),
                    name = item.optString("name", "Imported vault"),
                    treeUri = if (item.isNull("uri")) null else item.optString("uri"),
                )
            }
        }.getOrElse { mutableListOf() }
    }

    private fun writeMeta(metas: List<Meta>) {
        val array = JSONArray()
        for (meta in metas) {
            val item = JSONObject()
            item.put("id", meta.id)
            item.put("name", meta.name)
            item.put("uri", meta.treeUri ?: JSONObject.NULL)
            array.put(item)
        }
        preferences.edit().putString("vaults", array.toString()).apply()
    }

    private data class Meta(val id: String, val name: String, val treeUri: String?)

    private companion object {
        const val DEFAULT_ID = "default"
    }
}
