package com.dragonglass.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import uniffi.dragonglass.MobileVault
import java.io.File
import java.nio.file.Files

/**
 * Runs the generated UniFFI bindings against the host build of the Rust engine.
 *
 * This catches binding/API drift before the app reaches a device; the Android ABI build is
 * still verified by `scripts/build-native.ps1`.
 */
class FfiSmokeTest {
    @Test
    fun opensWritesSearchesAndRenders() {
        val root = Files.createTempDirectory("dragon-glass-ffi").toFile()
        try {
            File(root, "Hello.md").writeText("# Hello\n\nneedle")

            val vault = MobileVault.open(root.absolutePath)
            assertEquals(1, vault.notes().size)
            assertEquals("Hello", vault.notes().first().title)
            assertTrue(vault.search("needle", 10).isNotEmpty())

            vault.createNote("Second.md", "# Second\n\n## Child\n")
            assertEquals(2, vault.notes().size)
            assertTrue(vault.noteHtml("Second.md").contains("<h1>Second</h1>"))

            val headings = uniffi.dragonglass.headings(vault.noteSource("Second.md"))
            assertEquals(2, headings.size)
            assertEquals("Child", headings[1].text)
        } finally {
            root.deleteRecursively()
        }
    }
}
