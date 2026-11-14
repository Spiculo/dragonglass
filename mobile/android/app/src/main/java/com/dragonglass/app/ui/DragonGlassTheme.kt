package com.dragonglass.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

private val DarkColors = darkColorScheme(
    primary = Color(0xFF9DB8FF),
    onPrimary = Color(0xFF10203F),
    secondary = Color(0xFF8FD3C8),
    background = Color(0xFF0F1116),
    onBackground = Color(0xFFE4E7EE),
    surface = Color(0xFF14171E),
    onSurface = Color(0xFFE4E7EE),
    surfaceVariant = Color(0xFF222733),
    onSurfaceVariant = Color(0xFFB9C0CF),
    outline = Color(0xFF3A4150),
)

private val LightColors = lightColorScheme(
    primary = Color(0xFF2F5DA8),
    onPrimary = Color.White,
    secondary = Color(0xFF2F7A6D),
    background = Color(0xFFFAFAFC),
    onBackground = Color(0xFF1B1D22),
    surface = Color(0xFFFFFFFF),
    onSurface = Color(0xFF1B1D22),
    surfaceVariant = Color(0xFFF0F1F5),
    onSurfaceVariant = Color(0xFF454B58),
    outline = Color(0xFFC8CCD6),
)

/// `darkTheme` overrides the system choice; null follows the system.
@Composable
fun DragonGlassTheme(darkTheme: Boolean? = null, content: @Composable () -> Unit) {
    val dark = darkTheme ?: isSystemInDarkTheme()
    MaterialTheme(
        colorScheme = if (dark) DarkColors else LightColors,
        typography = Typography(),
        content = content,
    )
}
