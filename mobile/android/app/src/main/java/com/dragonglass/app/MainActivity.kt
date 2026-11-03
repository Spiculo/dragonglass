package com.dragonglass.app

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.runtime.Composable
import androidx.lifecycle.viewmodel.compose.viewModel
import com.dragonglass.app.ui.DragonGlassApp
import com.dragonglass.app.ui.DragonGlassTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            DragonGlassTheme {
                DragonGlassContent()
            }
        }
    }
}

@Composable
private fun DragonGlassContent() {
    val viewModel: DragonGlassViewModel = viewModel()
    DragonGlassApp(viewModel)
}
