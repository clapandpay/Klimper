// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on Android (clapandpay TASK-111.02): the coin jingle through a SoundPool
// (skipped when the ringer is on silent or vibrate, like the iOS silent switch), the haptic
// pattern the webview chose through the Vibrator, and a shake detector on the accelerometer
// that reports to Rust over a channel. Nothing plays or vibrates unless the app is resumed.
//
// The WAV bytes arrive from Rust base64-encoded once per tier per run and are written to the
// app's cache directory, because SoundPool loads from a path.
//
// Not yet compiled: there is no Android SDK on the development machine (docs/sound.md).

package com.clapandpay.klimper.jingle

import android.app.Activity
import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.media.AudioAttributes
import android.media.AudioManager
import android.media.SoundPool
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Base64
import android.util.Log
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.io.File
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

@InvokeArg
class ImpactArg {
    var atMs: Int = 0
    var strength: Float = 0f
}

@InvokeArg
class PlayArgs {
    lateinit var tier: String
    var wav: String? = null
    var impacts: List<ImpactArg> = emptyList()
}

@InvokeArg
class ShakeArgs {
    var enabled: Boolean = false
    var channel: Channel? = null
}

@TauriPlugin
class KlimperJinglePlugin(private val activity: Activity) : Plugin(activity), SensorEventListener {
    companion object {
        private const val TAG = "klimper.jingle"

        // A shake is several strong jolts in a short time, then a pause before the next counts.
        private const val JOLT_G = 1.6
        private const val JOLTS_NEEDED = 3
        private const val WINDOW_MS = 700L
        private const val COOLDOWN_MS = 1500L
    }

    private val main = Handler(Looper.getMainLooper())
    private var pool: SoundPool? = null
    private val sounds = HashMap<String, Int>()
    private val loaded = HashSet<Int>()

    private var sensors: SensorManager? = null
    private var shakeChannel: Channel? = null
    private var shakeEnabled = false
    private var listening = false
    private val peaks = ArrayList<Long>()
    private var peakIntensity = 0.0
    private var lastShake = 0L

    // Starts true: if the resume hook were ever not delivered, a silent app forever would be
    // the worse failure; the pause hook turns it off.
    @Volatile
    private var resumed = true

    override fun onResume() {
        super.onResume()
        resumed = true
        startMotionIfWanted()
    }

    override fun onPause() {
        super.onPause()
        resumed = false
        stopMotion()
    }

    // MARK: Commands

    @Command
    fun play(invoke: Invoke) {
        val args = invoke.parseArgs(PlayArgs::class.java)
        main.post {
            if (!resumed) {
                invoke.resolve() // background: a wallet in a pocket makes no sound on its own
                return@post
            }
            try {
                playOnMain(args)
                invoke.resolve()
            } catch (error: Exception) {
                invoke.reject("jingle: ${error.message}")
            }
        }
    }

    @Command
    fun setShakeEnabled(invoke: Invoke) {
        val args = invoke.parseArgs(ShakeArgs::class.java)
        main.post {
            shakeEnabled = args.enabled
            if (args.enabled) {
                shakeChannel = args.channel ?: shakeChannel
                startMotionIfWanted()
            } else {
                stopMotion()
                shakeChannel = null
            }
            invoke.resolve()
        }
    }

    // MARK: Sound and touch

    private fun playOnMain(args: PlayArgs) {
        val pool = this.pool ?: SoundPool.Builder()
            .setMaxStreams(2)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build()
            )
            .build()
            .also { p ->
                p.setOnLoadCompleteListener { _, id, status -> if (status == 0) loaded.add(id) }
                this.pool = p
            }
        args.wav?.let { wav ->
            val bytes = Base64.decode(wav, Base64.DEFAULT)
            val file = File(activity.cacheDir, "klimper-${args.tier}.wav")
            file.writeBytes(bytes)
            sounds[args.tier]?.let { pool.unload(it) }
            sounds[args.tier] = pool.load(file.path, 1)
        }
        val id = sounds[args.tier]
            ?: throw IllegalStateException("no sound for tier ${args.tier}; send it first")
        feel(args.impacts)
        // Silent or vibrate: the hand still feels it, the room hears nothing.
        val audio = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        if (audio.ringerMode != AudioManager.RINGER_MODE_NORMAL) return
        if (loaded.contains(id)) {
            pool.play(id, 1f, 1f, 1, 0, 1f)
        } else {
            // Still decoding (first play of this tier): play as soon as it is ready, once.
            pool.setOnLoadCompleteListener { p, done, status ->
                if (status == 0) loaded.add(done)
                if (done == id && status == 0) p.play(id, 1f, 1f, 1, 0, 1f)
            }
        }
    }

    private fun feel(impacts: List<ImpactArg>) {
        if (impacts.isEmpty()) return
        val vibrator = vibrator() ?: return
        if (!vibrator.hasVibrator()) return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            // One waveform: off until each impact, then a short pulse whose amplitude is the
            // strength. Pulses last 20 to 45 ms, heavier ones longer.
            val timings = ArrayList<Long>()
            val amplitudes = ArrayList<Int>()
            var cursor = 0L
            for (impact in impacts.sortedBy { it.atMs }) {
                val at = impact.atMs.toLong()
                val strength = max(0.1f, min(1f, impact.strength))
                val pulse = (20 + 25 * strength).toLong()
                timings.add(max(0L, at - cursor)); amplitudes.add(0)
                timings.add(pulse); amplitudes.add((strength * 255).toInt())
                cursor = at + pulse
            }
            vibrator.vibrate(VibrationEffect.createWaveform(timings.toLongArray(), amplitudes.toIntArray(), -1))
        } else {
            @Suppress("DEPRECATION")
            vibrator.vibrate(40)
        }
    }

    private fun vibrator(): Vibrator? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (activity.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as? VibratorManager)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            activity.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
        }

    // MARK: Shake

    private fun startMotionIfWanted() {
        if (!shakeEnabled || shakeChannel == null || listening || !resumed) return
        val manager = sensors ?: (activity.getSystemService(Context.SENSOR_SERVICE) as SensorManager).also { sensors = it }
        // Linear acceleration has gravity removed; fall back to the raw accelerometer.
        val sensor = manager.getDefaultSensor(Sensor.TYPE_LINEAR_ACCELERATION)
            ?: manager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
            ?: return
        peaks.clear()
        peakIntensity = 0.0
        listening = manager.registerListener(this, sensor, SensorManager.SENSOR_DELAY_GAME)
    }

    private fun stopMotion() {
        if (listening) sensors?.unregisterListener(this)
        listening = false
        peaks.clear()
        peakIntensity = 0.0
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    override fun onSensorChanged(event: SensorEvent) {
        val x = event.values[0].toDouble()
        val y = event.values[1].toDouble()
        val z = event.values[2].toDouble()
        var magnitude = sqrt(x * x + y * y + z * z) / SensorManager.GRAVITY_EARTH
        if (event.sensor.type == Sensor.TYPE_ACCELEROMETER) {
            // Raw accelerometer: remove gravity's 1 g by magnitude (good enough for a jolt).
            magnitude = kotlin.math.abs(magnitude - 1.0)
        }
        if (magnitude < JOLT_G) return
        val now = event.timestamp / 1_000_000 // ns to ms
        peaks.removeAll { now - it > WINDOW_MS }
        peaks.add(now)
        peakIntensity = max(peakIntensity, magnitude)
        if (peaks.size < JOLTS_NEEDED || now - lastShake < COOLDOWN_MS) return
        lastShake = now
        peaks.clear()
        // 1.6 g is the least that counts, 4 g is as hard as a hand shakes a phone.
        val intensity = min(1.0, max(0.0, (peakIntensity - JOLT_G) / (4.0 - JOLT_G)))
        peakIntensity = 0.0
        if (!resumed) return
        try {
            shakeChannel?.send(JSObject().put("intensity", intensity))
        } catch (error: Exception) {
            Log.w(TAG, "shake not delivered: ${error.message}")
        }
    }
}
