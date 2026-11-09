package io.superteem.voice

import android.os.Bundle
import android.view.View
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Android draws apps edge to edge, under the status bar, the navigation bar and the keyboard. The page
        // is kept between them instead, so nothing hides under the clock and the dictation controls at the
        // bottom of a note sit just above the keyboard while it is open.
        val content = findViewById<View>(android.R.id.content)
        ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
            val types = WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout() or WindowInsetsCompat.Type.ime()
            val edges = insets.getInsets(types)
            view.setPadding(edges.left, edges.top, edges.right, edges.bottom)
            WindowInsetsCompat.CONSUMED
        }
    }
}
