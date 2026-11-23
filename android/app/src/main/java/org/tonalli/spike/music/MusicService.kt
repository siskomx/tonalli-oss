package org.tonalli.spike.music

import android.app.*
import android.content.*
import android.content.pm.ServiceInfo
import android.media.*
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.graphics.drawable.Icon
import android.os.*
import org.tonalli.spike.R
import uniffi.tonalli_core.*

/** Explicit playback owns its decoder independently of the disposable WebView. */
class MusicService : Service() {
    private val thread = HandlerThread("Tonalli music")
    private lateinit var worker: Handler
    private lateinit var session: MediaSession
    private lateinit var audio: AudioManager
    private lateinit var focus: AudioFocusRequest
    private var player: MediaPlayer? = null
    private var source: ManagedAudioSource? = null
    private var track = PlaybackTrack(null, 0uL, 0.0)
    private var prepared = false
    private var wantPlay = false
    private var title = "Music"
    private var stopping = false
    private val attributes = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA)
        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC).build()
    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) { command(PlaybackAction.PAUSE, track) }
    }
    private val tick = object : Runnable {
        override fun run() {
            if (stopping) { return }
            guarded { publish(); save() }
            worker.postDelayed(this, 5000)
        }
    }
    override fun onCreate() {
        super.onCreate()
        thread.start(); worker = Handler(thread.looper)
        NativeMusic.service = this
        title = getString(R.string.music_title)
        audio = getSystemService(AudioManager::class.java)
        focus = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN).setAudioAttributes(attributes)
            .setOnAudioFocusChangeListener({ change ->
                when (change) {
                    AudioManager.AUDIOFOCUS_LOSS, AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> pause()
                    AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> player?.setVolume(0.2f, 0.2f)
                    AudioManager.AUDIOFOCUS_GAIN -> player?.setVolume(1f, 1f)
                }
            }, worker).build()
        session = MediaSession(this, "Tonalli music")
        session.setCallback(object : MediaSession.Callback() {
            override fun onPlay() { guarded { apply(PlaybackAction.PLAY, musicCurrent(NativeMusic.root(this@MusicService))) } }
            override fun onPause() { pause() }
            override fun onStop() { finish() }
            override fun onSkipToNext() { guarded { step(1) } }
            override fun onSkipToPrevious() { guarded { step(-1) } }
            override fun onSeekTo(pos: Long) {
                guarded { apply(PlaybackAction.SEEK, musicSeek(NativeMusic.root(this@MusicService), track.generation, pos.coerceAtLeast(0).toDouble() / 1000)) }
            }
        }, worker)
        session.isActive = true
        registerReceiver(noisy, IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY), RECEIVER_NOT_EXPORTED)
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.music_channel), NotificationManager.IMPORTANCE_LOW))
        worker.post(tick)
    }
    override fun onBind(intent: Intent): IBinder? = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(NOTIFICATION, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        val request = NativeMusic.take(intent?.getStringExtra("request"))
        if (request != null) command(request.first, request.second)
        else worker.post { guarded {
            when (intent?.action) {
                PLAY -> apply(PlaybackAction.PLAY, musicCurrent(NativeMusic.root(this)))
                PAUSE -> pause()
                NEXT -> step(1)
                PREVIOUS -> step(-1)
                else -> finish()
            }
        } }
        return START_NOT_STICKY
    }
    internal fun command(action: PlaybackAction, selected: PlaybackTrack) { worker.post { guarded { apply(action, selected) } } }
    private fun guarded(operation: () -> Unit) {
        try { operation() } catch (_: Exception) { fail(getString(R.string.music_unavailable)) }
    }
    private fun apply(action: PlaybackAction, selected: PlaybackTrack) {
        if (stopping) return
        if (action == PlaybackAction.STOP) { finish(); return }
        if (action == PlaybackAction.PAUSE) { pause(); return }
        val current = musicCurrent(NativeMusic.root(this))
        if (current.id != selected.id || current.generation != selected.generation) return
        if (selected.id == null) {
            finish()
            track = selected
            NativeMusic.latest = NativeMusic.latest.copy(id = null, generation = selected.generation, seconds = 0.0, duration = 0.0)
            return
        }
        val play = action == PlaybackAction.PLAY || (action == PlaybackAction.SEEK && wantPlay)
        if (track.id == selected.id && track.generation == selected.generation && player != null) {
            wantPlay = play
            if (prepared && play) startPlayer()
            publish(); return
        }
        // A seek already committed a new generation: never save the old decoder over it.
        save(); releasePlayer()
        track = selected; wantPlay = play
        NativeMusic.latest = NativeMusic.latest.copy(error = null)
        val handle = musicSourceOpen(NativeMusic.root(this), checkNotNull(selected.id))
        title = handle.name()
        val input = ManagedAudioSource(handle)
        source = input
        val decoder = MediaPlayer()
        player = decoder
        decoder.setAudioAttributes(attributes)
        decoder.setWakeMode(this, PowerManager.PARTIAL_WAKE_LOCK)
        decoder.setDataSource(input)
        decoder.setOnPreparedListener {
            if (player !== decoder || stopping) return@setOnPreparedListener
            if (action != PlaybackAction.SEEK && decoder.duration > 0 && track.seconds >= decoder.duration.toDouble() / 1000) {
                track = track.copy(seconds = 0.0)
                ready(decoder)
            } else if (track.seconds > 0) decoder.seekTo((track.seconds * 1000).toLong(), MediaPlayer.SEEK_CLOSEST)
            else ready(decoder)
        }
        decoder.setOnSeekCompleteListener { if (player === decoder && !stopping) ready(decoder) }
        decoder.setOnCompletionListener { if (player === decoder && !stopping) guarded { save(); step(1) } }
        decoder.setOnErrorListener { _, _, _ ->
            if (player === decoder && !stopping) fail(getString(R.string.music_format))
            true
        }
        publish(); decoder.prepareAsync()
    }
    private fun ready(decoder: MediaPlayer) {
        prepared = true
        if (wantPlay) startPlayer()
        publish()
    }
    private fun startPlayer() {
        if (audio.requestAudioFocus(focus) != AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
            wantPlay = false
            NativeMusic.latest = NativeMusic.latest.copy(error = getString(R.string.music_focus))
            return
        }
        player?.start()
        NativeMusic.latest = NativeMusic.latest.copy(error = null)
    }
    private fun pause() {
        wantPlay = false
        if (prepared) player?.pause()
        save(); audio.abandonAudioFocusRequest(focus); publish()
    }
    private fun step(delta: Int) {
        save()
        val next = musicStep(NativeMusic.root(this), track.generation, delta)
        apply(if (wantPlay) PlaybackAction.PLAY else PlaybackAction.LOAD, next)
    }
    private fun save() {
        if (!prepared) return
        val seconds = (player?.currentPosition ?: return).coerceAtLeast(0).toDouble() / 1000
        try { musicSavePosition(NativeMusic.root(this), track.generation, seconds) }
        catch (_: Exception) { NativeMusic.latest = NativeMusic.latest.copy(error = getString(R.string.music_save_failed)) }
    }
    private fun publish() {
        val seconds = if (prepared) (player?.currentPosition ?: 0).coerceAtLeast(0).toDouble() / 1000 else track.seconds
        val duration = if (prepared) (player?.duration ?: 0).coerceAtLeast(0).toDouble() / 1000 else 0.0
        val playing = prepared && player?.isPlaying == true
        NativeMusic.latest = NativeMusic.latest.copy(available = true, playing = playing, loading = player != null && !prepared,
            id = track.id, generation = track.generation, seconds = seconds, duration = duration)
        session.setMetadata(MediaMetadata.Builder().putString(MediaMetadata.METADATA_KEY_TITLE, title)
            .putLong(MediaMetadata.METADATA_KEY_DURATION, (duration * 1000).toLong()).build())
        session.setPlaybackState(PlaybackState.Builder().setActions(PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
            PlaybackState.ACTION_STOP or PlaybackState.ACTION_SKIP_TO_NEXT or PlaybackState.ACTION_SKIP_TO_PREVIOUS or PlaybackState.ACTION_SEEK_TO)
            .setState(if (playing) PlaybackState.STATE_PLAYING else if (prepared) PlaybackState.STATE_PAUSED else PlaybackState.STATE_BUFFERING,
                (seconds * 1000).toLong(), if (playing) 1f else 0f).build())
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification())
    }
    private fun notification(): Notification {
        fun action(label: String, verb: String, icon: Int): Notification.Action {
            val intent = Intent(this, MusicService::class.java).setAction(verb)
            val pending = PendingIntent.getForegroundService(this, verb.hashCode(), intent, PendingIntent.FLAG_IMMUTABLE)
            return Notification.Action.Builder(Icon.createWithResource(this, icon), label, pending).build()
        }
        val open = PendingIntent.getActivity(this, 12, Intent(this, org.tonalli.spike.MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val playing = NativeMusic.latest.playing
        return Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.ic_media_play)
            .setContentTitle(title).setContentText(getString(if (playing) R.string.music_playing else R.string.music_paused))
            .setContentIntent(open).setOngoing(playing).setOnlyAlertOnce(true)
            .setStyle(Notification.MediaStyle().setMediaSession(session.sessionToken).setShowActionsInCompactView(0, 1, 2))
            .addAction(action(getString(R.string.music_previous), PREVIOUS, android.R.drawable.ic_media_previous))
            .addAction(action(getString(if (playing) R.string.music_pause else R.string.music_play), if (playing) PAUSE else PLAY, if (playing) android.R.drawable.ic_media_pause else android.R.drawable.ic_media_play))
            .addAction(action(getString(R.string.music_next), NEXT, android.R.drawable.ic_media_next))
            .addAction(action(getString(R.string.music_stop), STOP, android.R.drawable.ic_menu_close_clear_cancel)).build()
    }
    private fun releasePlayer() {
        prepared = false
        player?.release(); player = null
        source?.close(); source = null
    }
    private fun fail(message: String) {
        wantPlay = false; releasePlayer(); audio.abandonAudioFocusRequest(focus)
        NativeMusic.latest = NativeMusic.latest.copy(playing = false, loading = false, error = message)
        finish()
    }
    private fun finish() {
        if (stopping) return
        save(); stopping = true; wantPlay = false
        releasePlayer(); audio.abandonAudioFocusRequest(focus)
        NativeMusic.latest = NativeMusic.latest.copy(playing = false, loading = false)
        stopForeground(STOP_FOREGROUND_REMOVE); stopSelf()
    }
    override fun onDestroy() {
        if (NativeMusic.service === this) NativeMusic.service = null
        unregisterReceiver(noisy)
        worker.post { finish(); session.release(); worker.removeCallbacksAndMessages(null); thread.quitSafely() }
        super.onDestroy()
    }
    companion object {
        private const val CHANNEL = "music"
        private const val NOTIFICATION = 12
        private const val PLAY = "music.play"
        private const val PAUSE = "music.pause"
        private const val STOP = "music.stop"
        private const val NEXT = "music.next"
        private const val PREVIOUS = "music.previous"
    }
}
