package org.tonalli.spike.ai

import android.app.Activity
import android.os.Bundle
import android.widget.TextView

/** Normal ffi-only APK has no references to optional generated AI symbols. */
class AiProbeActivity : Activity() {
    override fun onCreate(state: Bundle?) {
        super.onCreate(state)
        setContentView(TextView(this).apply {
            setPadding(24, 24, 24, 24)
            text = "This normal build omits optional local AI. The M1 AI probe requires an explicit enableAi build. Offline library, maps and notes remain available."
        })
    }
}
