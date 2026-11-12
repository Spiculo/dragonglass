package com.dragonglass.app

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.dragonglass.app.sync.GitSyncManager
import org.eclipse.jgit.api.Git
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Loads JGit on a real Android runtime, which the JVM unit test cannot prove.
 */
@RunWith(AndroidJUnit4::class)
class GitAndroidTest {
    @Test
    fun jgitInitializesAndPushesToAFileRemote() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val root = File(context.cacheDir, "git-android-${System.currentTimeMillis()}")
        try {
            val remote = File(root, "remote.git")
            Git.init().setBare(true).setDirectory(remote).call().use { }

            val vault = File(root, "vault").apply { mkdirs() }
            File(vault, "Note.md").writeText("# Note\n")

            val manager = GitSyncManager(
                vault,
                remote.toURI().toString(),
                "main",
                null,
                "Test User",
                "test@example.com",
            )
            manager.initRemote()

            assertTrue(File(remote, "refs/heads/main").exists())
        } finally {
            root.deleteRecursively()
        }
    }
}
