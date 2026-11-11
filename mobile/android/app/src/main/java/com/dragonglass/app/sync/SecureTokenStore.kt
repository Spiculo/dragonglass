package com.dragonglass.app.sync

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.nio.ByteBuffer
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Stores the GitHub token in Android Keystore-backed AES/GCM encryption.
 *
 * The token never touches the vault, git config or logs. If the key is invalidated (for
 * example by a lock-screen change), the stored blob is discarded and the user re-enters it.
 */
class SecureTokenStore(context: Context) {
    private val preferences =
        context.getSharedPreferences("dragon_glass_git_auth", Context.MODE_PRIVATE)

    private val keyStore: KeyStore
        get() = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }

    fun hasToken(): Boolean = load() != null

    fun save(token: String) {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, secretKey())
        val iv = cipher.iv
        val ciphertext = cipher.doFinal(token.toByteArray(Charsets.UTF_8))
        val blob = ByteBuffer.allocate(1 + iv.size + ciphertext.size)
            .put(iv.size.toByte())
            .put(iv)
            .put(ciphertext)
            .array()
        preferences.edit()
            .putString(KEY_TOKEN, Base64.encodeToString(blob, Base64.NO_WRAP))
            .apply()
    }

    fun load(): String? {
        val encoded = preferences.getString(KEY_TOKEN, null) ?: return null
        return try {
            val blob = Base64.decode(encoded, Base64.NO_WRAP)
            if (blob.isEmpty()) return null
            val ivSize = blob[0].toInt()
            if (ivSize <= 0 || 1 + ivSize >= blob.size) return null
            val iv = blob.copyOfRange(1, 1 + ivSize)
            val ciphertext = blob.copyOfRange(1 + ivSize, blob.size)
            val cipher = Cipher.getInstance(TRANSFORMATION)
            cipher.init(Cipher.DECRYPT_MODE, secretKey(), GCMParameterSpec(128, iv))
            String(cipher.doFinal(ciphertext), Charsets.UTF_8)
        } catch (_: Exception) {
            clear()
            null
        }
    }

    fun clear() {
        preferences.edit().remove(KEY_TOKEN).apply()
        runCatching { keyStore.deleteEntry(KEY_ALIAS) }
    }

    private fun secretKey(): SecretKey {
        val store = keyStore
        val existing = store.getEntry(KEY_ALIAS, null) as? KeyStore.SecretKeyEntry
        if (existing != null) return existing.secretKey

        val generator =
            KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
        generator.init(
            KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build(),
        )
        return generator.generateKey()
    }

    private companion object {
        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val KEY_ALIAS = "dragon-glass-github-token"
        const val KEY_TOKEN = "token"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
    }
}
