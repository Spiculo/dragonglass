package com.dragonglass.app.sync

import org.eclipse.jgit.api.Git
import org.eclipse.jgit.api.RebaseCommand
import org.eclipse.jgit.lib.BranchTrackingStatus
import org.eclipse.jgit.lib.PersonIdent
import org.eclipse.jgit.transport.CredentialsProvider
import org.eclipse.jgit.transport.RefSpec
import org.eclipse.jgit.transport.RemoteRefUpdate
import org.eclipse.jgit.transport.URIish
import org.eclipse.jgit.transport.UsernamePasswordCredentialsProvider
import java.io.File
import java.time.LocalDateTime
import java.time.format.DateTimeFormatter

data class GitStatus(
    val isRepo: Boolean,
    val hasRemote: Boolean,
    val branch: String,
    val changed: Int,
    val untracked: Int,
    val ahead: Int,
    val behind: Int,
    val conflicts: List<String>,
) {
    val hasLocalWork: Boolean
        get() = changed > 0 || untracked > 0 || conflicts.isNotEmpty()
}

data class GitSyncResult(
    val committed: Boolean,
    val pulled: Boolean,
    val pushed: Boolean,
    val status: GitStatus,
    val conflicts: List<String>,
)

class GitSyncException(
    message: String,
    val conflicts: List<String> = emptyList(),
) : Exception(message)

/**
 * One-commit git sync for a vault.
 *
 * `sync()` batches every pending note change into one commit, pulls with rebase (so the
 * local change stays a single commit), then pushes. Credentials are supplied per network
 * command and are never written into the repository.
 */
class GitSyncManager(
    private val vaultDir: File,
    private val remoteUrl: String,
    private val branch: String,
    private val token: String?,
    private val authorName: String = "Dragon Glass",
    private val authorEmail: String = "dragon-glass@localhost",
) {
    fun isRepo(): Boolean = File(vaultDir, ".git").exists()

    fun status(): GitStatus {
        if (!isRepo()) {
            return GitStatus(
                isRepo = false,
                hasRemote = false,
                branch = branch,
                changed = 0,
                untracked = 0,
                ahead = 0,
                behind = 0,
                conflicts = emptyList(),
            )
        }
        return Git.open(vaultDir).use { git ->
            val repo = git.repository
            val current = runCatching { repo.branch }.getOrNull().orEmpty().ifBlank { branch }
            val status = git.status().call()
            val changed =
                status.added.size + status.modified.size + status.removed.size + status.missing.size
            val remoteConfigured =
                repo.config.getString("remote", "origin", "url")?.isNotBlank() == true
            val tracking =
                if (remoteConfigured) {
                    runCatching { BranchTrackingStatus.of(repo, current) }.getOrNull()
                } else {
                    null
                }
            GitStatus(
                isRepo = true,
                hasRemote = remoteConfigured,
                branch = current,
                changed = changed,
                untracked = status.untracked.size,
                ahead = tracking?.aheadCount ?: 0,
                behind = tracking?.behindCount ?: 0,
                conflicts = status.conflicting.toList().sorted(),
            )
        }
    }

    /**
     * Initialize a new repository in the vault and push its first commit.
     *
     * Use this only for an empty remote. If the remote already has a branch, use
     * [cloneIntoVault] instead; this method refuses to merge unrelated histories.
     */
    fun initRemote(): GitStatus {
        require(remoteUrl.isNotBlank()) { "A remote URL is required" }
        if (isRepo()) {
            throw GitSyncException("This vault is already a git repository")
        }

        Git.init()
            .setDirectory(vaultDir)
            .setInitialBranch(branch)
            .call()
            .use { git ->
                configureRemote(git)
                if (remoteHasBranch(git)) {
                    throw GitSyncException(
                        "The remote already has a '$branch' branch. Link the existing repository instead.",
                    )
                }
                ensureGitignore()
                commitAll(git, initialMessage())
                push(git)
            }
        return status()
    }

    /**
     * Replace the local vault with a clone of the remote.
     *
     * The old vault is renamed to `<vault>.backup-<timestamp>` on success, so local notes
     * are recoverable if the user linked the wrong repository.
     */
    fun cloneIntoVault(): GitStatus {
        require(remoteUrl.isNotBlank()) { "A remote URL is required" }
        val parent = vaultDir.parentFile
            ?: throw GitSyncException("The vault has no parent directory")
        val stamp = System.currentTimeMillis()
        val temp = File(parent, "${vaultDir.name}.clone-$stamp")
        val backup = File(parent, "${vaultDir.name}.backup-$stamp")

        try {
            Git.cloneRepository()
                .setURI(remoteUrl)
                .setDirectory(temp)
                .setBranch(branch)
                .setCredentialsProvider(credentials())
                .call()
                .use { }

            if (!vaultDir.renameTo(backup)) {
                throw GitSyncException("Could not move the existing vault aside")
            }
            if (!temp.renameTo(vaultDir)) {
                backup.renameTo(vaultDir)
                throw GitSyncException("Could not move the cloned repository into place")
            }
        } catch (error: Exception) {
            temp.deleteRecursively()
            throw error
        }
        return status()
    }

    /**
     * Commit all local changes as one commit, pull with rebase, then push.
     */
    fun sync(): GitSyncResult {
        if (!isRepo()) {
            throw GitSyncException("Initialize or link a repository first")
        }
        return Git.open(vaultDir).use { git ->
            configureRemote(git)
            ensureGitignore()
            val committed = commitAll(git, syncMessage())

            var pulled = false
            if (remoteHasBranch(git)) {
                pulled = pullRebase(git)
            }
            val conflicts = git.status().call().conflicting.toList().sorted()
            if (conflicts.isNotEmpty()) {
                throw GitSyncException("Sync stopped: resolve conflicts first", conflicts)
            }

            push(git)
            GitSyncResult(
                committed = committed,
                pulled = pulled,
                pushed = true,
                status = status(),
                conflicts = emptyList(),
            )
        }
    }

    /**
     * Abort an in-progress rebase, restoring the working tree to the local commit.
     */
    fun abortRebase() {
        if (!isRepo()) return
        Git.open(vaultDir).use { git ->
            runCatching {
                git.rebase().setOperation(RebaseCommand.Operation.ABORT).call()
            }
        }
    }

    private fun configureRemote(git: Git) {
        val config = git.repository.config
        if (config.getString("remote", "origin", "url") == null) {
            git.remoteAdd()
                .setName("origin")
                .setUri(URIish(remoteUrl))
                .call()
        } else {
            config.setString("remote", "origin", "url", remoteUrl)
        }
        config.setString("branch", branch, "remote", "origin")
        config.setString("branch", branch, "merge", "refs/heads/$branch")
        config.save()
    }

    private fun ensureGitignore() {
        val ignore = File(vaultDir, ".gitignore")
        if (!ignore.exists()) {
            ignore.writeText(
                """
                .dragon-glass/
                .DS_Store
                Thumbs.db
                *.tmp
                """.trimIndent() + "\n",
            )
        }
    }

    private fun commitAll(git: Git, message: String): Boolean {
        git.add().addFilepattern(".").call()
        git.add().setUpdate(true).addFilepattern(".").call()
        if (!git.status().call().hasUncommittedChanges()) {
            return false
        }
        val author = PersonIdent(authorName, authorEmail)
        git.commit()
            .setMessage(message)
            .setAuthor(author)
            .setCommitter(author)
            .call()
        return true
    }

    private fun pullRebase(git: Git): Boolean {
        val before = git.repository.resolve("HEAD")?.name
        val result =
            git.pull()
                .setRemote("origin")
                .setRemoteBranchName(branch)
                .setRebase(true)
                .setCredentialsProvider(credentials())
                .call()
        if (!result.isSuccessful) {
            val conflicts = git.status().call().conflicting.toList()
            val reason = result.rebaseResult?.status?.name ?: "pull failed"
            throw GitSyncException("Pull stopped ($reason)", conflicts)
        }
        return git.repository.resolve("HEAD")?.name != before
    }

    private fun push(git: Git) {
        val results =
            git.push()
                .setRemote("origin")
                .setCredentialsProvider(credentials())
                .setRefSpecs(RefSpec("refs/heads/$branch:refs/heads/$branch"))
                .call()

        val rejected = results
            .flatMap { it.remoteUpdates }
            .filter {
                it.status == RemoteRefUpdate.Status.REJECTED_NONFASTFORWARD ||
                    it.status == RemoteRefUpdate.Status.REJECTED_REMOTE_CHANGED ||
                    it.status == RemoteRefUpdate.Status.REJECTED_OTHER_REASON
            }
        if (rejected.isNotEmpty()) {
            val detail = rejected.joinToString { update ->
                "${update.remoteName}: ${update.status}${update.message?.let { " ($it)" } ?: ""}"
            }
            throw GitSyncException("Push rejected: $detail")
        }
    }

    private fun remoteHasBranch(git: Git): Boolean {
        return runCatching {
            git.lsRemote()
                .setRemote("origin")
                .setCredentialsProvider(credentials())
                .setHeads(true)
                .call()
                .any { it.name == "refs/heads/$branch" }
        }.getOrDefault(false)
    }

    private fun credentials(): CredentialsProvider? {
        val value = token?.trim().orEmpty()
        if (value.isEmpty()) return null
        return UsernamePasswordCredentialsProvider("x-access-token", value)
    }

    private fun initialMessage(): String = "Dragon Glass initial commit"

    private fun syncMessage(): String {
        val stamp = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyy-MM-dd HH:mm"))
        return "Dragon Glass sync $stamp"
    }
}
