package com.dragonglass.app

import com.dragonglass.app.sync.GitSyncManager
import com.dragonglass.app.sync.PullOutcome
import org.eclipse.jgit.api.Git
import org.eclipse.jgit.lib.PersonIdent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import java.nio.file.Files

class GitSyncTest {
    private val author = PersonIdent("Test", "test@example.com")

    @Test
    fun initAndSyncPushASingleCommitForAllChanges() {
        val root = Files.createTempDirectory("dragon-glass-git").toFile()
        try {
            val remote = File(root, "remote.git")
            Git.init().setBare(true).setDirectory(remote).call().use { }

            val vault = File(root, "vault").apply { mkdirs() }
            File(vault, "Welcome.md").writeText("# Welcome\n")

            val manager = GitSyncManager(
                vault,
                remote.toURI().toString(),
                "main",
                null,
                "Test User",
                "test@example.com",
            )
            manager.initRemote()
            assertTrue(manager.status().isRepo)

            val clone = File(root, "clone")
            Git.cloneRepository()
                .setURI(remote.toURI().toString())
                .setDirectory(clone)
                .call()
                .use { }
            assertTrue(File(clone, "Welcome.md").exists())

            File(vault, "A.md").writeText("a\n")
            File(vault, "B.md").writeText("b\n")
            val synced = manager.sync()
            assertTrue(synced.committed)
            assertEquals(0, manager.status().ahead)

            val commits = Git.open(vault).use { it.log().call().toList() }
            assertEquals(2, commits.size) // initial commit + one sync commit for both files
            assertEquals("Test User", commits[0].authorIdent.name)
            assertEquals("test@example.com", commits[0].authorIdent.emailAddress)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun cloneReplacesTheLocalVaultAndKeepsABackup() {
        val root = Files.createTempDirectory("dragon-glass-git-clone").toFile()
        try {
            val remote = File(root, "remote.git")
            Git.init().setBare(true).setDirectory(remote).call().use { }

            val seed = File(root, "seed")
            Git.cloneRepository()
                .setURI(remote.toURI().toString())
                .setDirectory(seed)
                .call()
                .use { git ->
                    File(seed, "Remote.md").writeText("from github\n")
                    git.add().addFilepattern(".").call()
                    git.commit()
                        .setMessage("seed")
                        .setAuthor(author)
                        .setCommitter(author)
                        .call()
                    git.push().call()
                }

            val vault = File(root, "vault").apply { mkdirs() }
            File(vault, "Local.md").writeText("only local\n")

            val manager = GitSyncManager(
                vault,
                remote.toURI().toString(),
                "main",
                null,
                "Test User",
                "test@example.com",
            )
            manager.cloneIntoVault()

            assertTrue(File(vault, "Remote.md").exists())
            assertFalse(File(vault, "Local.md").exists())
            assertTrue(root.listFiles().orEmpty().any { it.name.startsWith("vault.backup-") })
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun syncPullsRemoteChanges() {
        val root = Files.createTempDirectory("dragon-glass-git-pull").toFile()
        try {
            val remote = File(root, "remote.git")
            Git.init().setBare(true).setDirectory(remote).call().use { }

            val vault = File(root, "vault").apply { mkdirs() }
            File(vault, "Welcome.md").writeText("# Welcome\n")
            val manager = GitSyncManager(
                vault,
                remote.toURI().toString(),
                "main",
                null,
                "Test User",
                "test@example.com",
            )
            manager.initRemote()

            val other = File(root, "other")
            Git.cloneRepository()
                .setURI(remote.toURI().toString())
                .setDirectory(other)
                .call()
                .use { git ->
                    File(other, "Remote.md").writeText("from remote\n")
                    git.add().addFilepattern(".").call()
                    git.commit()
                        .setMessage("remote change")
                        .setAuthor(author)
                        .setCommitter(author)
                        .call()
                    git.push().call()
                }

            val result = manager.sync()
            assertTrue(result.pulled)
            assertTrue(File(vault, "Remote.md").exists())
            assertFalse(manager.status().hasLocalWork)
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun openPullFastForwardsRemoteChangesWhenClean() {
        val root = Files.createTempDirectory("dragon-glass-git-open-pull").toFile()
        try {
            val remote = File(root, "remote.git")
            Git.init().setBare(true).setDirectory(remote).call().use { }

            val vault = File(root, "vault").apply { mkdirs() }
            File(vault, "Welcome.md").writeText("# Welcome\n")
            val manager = GitSyncManager(
                vault,
                remote.toURI().toString(),
                "main",
                null,
                "Test User",
                "test@example.com",
            )
            manager.initRemote()
            assertEquals(PullOutcome.UpToDate, manager.fetchAndPullFastForward())

            val other = File(root, "other")
            Git.cloneRepository()
                .setURI(remote.toURI().toString())
                .setDirectory(other)
                .call()
                .use { git ->
                    File(other, "Remote.md").writeText("from remote\n")
                    git.add().addFilepattern(".").call()
                    git.commit()
                        .setMessage("remote change")
                        .setAuthor(author)
                        .setCommitter(author)
                        .call()
                    git.push().call()
                }

            assertEquals(PullOutcome.Updated, manager.fetchAndPullFastForward())
            assertTrue(File(vault, "Remote.md").exists())
        } finally {
            root.deleteRecursively()
        }
    }

    @Test
    fun openPullSkipsUncommittedChanges() {
        val root = Files.createTempDirectory("dragon-glass-git-open-dirty").toFile()
        try {
            val remote = File(root, "remote.git")
            Git.init().setBare(true).setDirectory(remote).call().use { }

            val vault = File(root, "vault").apply { mkdirs() }
            File(vault, "Welcome.md").writeText("# Welcome\n")
            val manager = GitSyncManager(
                vault,
                remote.toURI().toString(),
                "main",
                null,
                "Test User",
                "test@example.com",
            )
            manager.initRemote()

            File(vault, "Draft.md").writeText("not committed\n")
            assertEquals(PullOutcome.LocalChanges, manager.fetchAndPullFastForward())
        } finally {
            root.deleteRecursively()
        }
    }
}
