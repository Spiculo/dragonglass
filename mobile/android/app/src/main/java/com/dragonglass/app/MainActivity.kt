package com.dragonglass.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import com.dragonglass.app.ui.DragonGlassApp
import com.dragonglass.app.ui.DragonGlassTheme

class MainActivity : ComponentActivity() {
    private val viewModel: DragonGlassViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            DragonGlassTheme {
                DragonGlassApp(viewModel)
            }
        }
    }

    override fun onStop() {
        super.onStop()
        // Backgrounding the app writes a recovery draft for the open note.
        viewModel.flushDraft()
    }
}
