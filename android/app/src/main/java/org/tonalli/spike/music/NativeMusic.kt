package org.tonalli.spike.music

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper
import java.io.File
import java.util.UUID
import org.tonalli.spike.R
import uniffi.tonalli_core.*

internal object NativeMusic {
    private val main = Handler(Looper.getMainLooper())
    private val pending = mutableMapOf<String, Pair<PlaybackAction, PlaybackTrack>>()
    var service: MusicService? = null // Main looper only.
    @Volatile var latest = PlaybackStatus(true, false, false, null, 0uL, 0.0, 0.0, null)
    fun root(context: Context) = File(context.filesDir, "core").absolutePath
    fun attach(context: Context, server: LoopbackHandle) {
        val app = context.applicationContext
        serverAttachMusic(server, object : MusicScheduler {
            override fun status(): PlaybackStatus = latest
            override fun control(action: PlaybackAction, track: PlaybackTrack) {
                main.post {
                    val active = service
                    if (active != null) active.command(action, track)
                    else if (action == PlaybackAction.PLAY) {
                        val token = UUID.randomUUID().toString()
                        pending[token] = action to track
                        latest = latest.copy(loading = true, error = null)
                        try { app.startForegroundService(Intent(app, MusicService::class.java).putExtra("request", token)) }
                        catch (_: RuntimeException) {
                            pending.remove(token)
                            latest = latest.copy(loading = false, error = app.getString(R.string.music_retry))
                        }
                    } else latest = latest.copy(playing = false, loading = false, id = track.id,
                        generation = track.generation, seconds = track.seconds, duration = 0.0, error = null)
                }
            }
        })
    }
    fun take(token: String?) = pending.remove(token)
}
