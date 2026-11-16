package com.stabilizernano.app.ui.theme

import android.os.Build
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

// Zenith-inspired elevated slate/gray surface hierarchy (never pure #000000 black).
val SlateBackgroundDark = Color(0xFF121417)
val SlateSurfaceLowestDark = Color(0xFF0E1013)
val SlateSurfaceLowDark = Color(0xFF181B20)
val SlateSurfaceCardDark = Color(0xFF1E2127)
val SlateSurfaceElevatedDark = Color(0xFF262A32)
val SlateSurfaceHighestDark = Color(0xFF303540)
val SlateOutlineDark = Color(0xFF363C48)

private val ExpressiveDarkColors = darkColorScheme(
    primary = Color(0xFF82D3FF),
    onPrimary = Color(0xFF00344A),
    primaryContainer = Color(0xFF164A66),
    onPrimaryContainer = Color(0xFFCBEBFF),
    secondary = Color(0xFFB7C9D9),
    onSecondary = Color(0xFF22323F),
    secondaryContainer = Color(0xFF344350),
    onSecondaryContainer = Color(0xFFD5E5F5),
    tertiary = Color(0xFF5CE0B8),
    onTertiary = Color(0xFF00382B),
    tertiaryContainer = Color(0xFF134D3C),
    onTertiaryContainer = Color(0xFF8EFAD6),
    error = Color(0xFFFFB4AB),
    onError = Color(0xFF690005),
    errorContainer = Color(0xFF7D1F20),
    onErrorContainer = Color(0xFFFFDAD6),
    background = SlateBackgroundDark,
    onBackground = Color(0xFFE3E6EC),
    surface = SlateBackgroundDark,
    onSurface = Color(0xFFE3E6EC),
    surfaceVariant = SlateSurfaceElevatedDark,
    onSurfaceVariant = Color(0xFFB8C0CC),
    surfaceContainerLowest = SlateSurfaceLowestDark,
    surfaceContainerLow = SlateSurfaceLowDark,
    surfaceContainer = SlateSurfaceCardDark,
    surfaceContainerHigh = SlateSurfaceElevatedDark,
    surfaceContainerHighest = SlateSurfaceHighestDark,
    outline = Color(0xFF6C7484),
    outlineVariant = SlateOutlineDark
)

private val ExpressiveLightColors = lightColorScheme(
    primary = Color(0xFF006490),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFCBE6FF),
    onPrimaryContainer = Color(0xFF001E2F),
    secondary = Color(0xFF4F606F),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFD3E4F5),
    onSecondaryContainer = Color(0xFF0C1D2A),
    tertiary = Color(0xFF006B54),
    onTertiary = Color(0xFFFFFFFF),
    tertiaryContainer = Color(0xFF88F8D3),
    onTertiaryContainer = Color(0xFF002018),
    error = Color(0xFFBA1A1A),
    errorContainer = Color(0xFFFFDAD6),
    onErrorContainer = Color(0xFF410002),
    background = Color(0xFFF3F6FA),
    onBackground = Color(0xFF181C20),
    surface = Color(0xFFF3F6FA),
    onSurface = Color(0xFF181C20),
    surfaceVariant = Color(0xFFE1E6EE),
    onSurfaceVariant = Color(0xFF424852),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFF7F9FC),
    surfaceContainer = Color(0xFFECEFF5),
    surfaceContainerHigh = Color(0xFFE4E8F0),
    surfaceContainerHighest = Color(0xFFDCE1EB),
    outline = Color(0xFF737985),
    outlineVariant = Color(0xFFC5CBD6)
)

@Composable
fun StabilizerTheme(
    darkTheme: Boolean = isSystemInDarkTheme(),
    dynamicColor: Boolean = true,
    content: @Composable () -> Unit
) {
    val colorScheme = when {
        dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S -> {
            val context = LocalContext.current
            if (darkTheme) {
                dynamicDarkColorScheme(context).copy(
                    background = SlateBackgroundDark,
                    surface = SlateBackgroundDark,
                    surfaceContainerLowest = SlateSurfaceLowestDark,
                    surfaceContainerLow = SlateSurfaceLowDark,
                    surfaceContainer = SlateSurfaceCardDark,
                    surfaceContainerHigh = SlateSurfaceElevatedDark,
                    surfaceContainerHighest = SlateSurfaceHighestDark,
                    outlineVariant = SlateOutlineDark
                )
            } else {
                dynamicLightColorScheme(context).copy(
                    background = Color(0xFFF3F6FA),
                    surface = Color(0xFFF3F6FA),
                    surfaceContainer = Color(0xFFECEFF5),
                    surfaceContainerHigh = Color(0xFFE4E8F0)
                )
            }
        }
        darkTheme -> ExpressiveDarkColors
        else -> ExpressiveLightColors
    }

    MaterialTheme(
        colorScheme = colorScheme,
        content = content
    )
}
