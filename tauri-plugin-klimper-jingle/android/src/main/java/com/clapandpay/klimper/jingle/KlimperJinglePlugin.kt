// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on Android (clapandpay TASK-111.02), the same two moments as the Swift:
// `play` rings the tier's ding once (SoundPool; skipped when the ringer is on silent or vibrate,
// like the iOS silent switch); `setShake` arms the accelerometer, and a shake loops the tier's
// rattle for as long as jolts keep coming, with a vibration pulse per jolt, stopping after a
// few hundred milliseconds of stillness. Nothing plays or vibrates unless the app is resumed.
//
// Sound bytes arrive from Rust base64-encoded once per sound per run and are written to the
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
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sqrt

@InvokeArg
class PlayArgs {
    lateinit var tier: String
    var wav: String? = null
}

@InvokeArg
class ShakeArgs {
    var enabled: Boolean = false
    lateinit var tier: String
    var wav: String? = null
}

@TauriPlugin
class KlimperJinglePlugin(private val activity: Activity) : Plugin(activity), SensorEventListener {
    companion object {
        // A jolt is acceleration (gravity removed) above this, in g. Two jolts within the
        // window start the rattle; it stops after STILL_MS without one.
        private const val JOLT_G = 1.3
        private const val WINDOW_MS = 450L
        private const val STILL_MS = 350L
        private const val TAP_SPACING_MS = 70L
    }

    private val main = Handler(Looper.getMainLooper())
    private var pool: SoundPool? = null
    private val dings = HashMap<String, Int>()
    private val rattles = HashMap<String, Int>()
    private val loaded = HashSet<Int>()

    private var shakeTier = "coin"
    private var shakeEnabled = false
    private var rattleStream = 0
    private var stopRattle: Runnable? = null
    private var lastTap = 0L

    private var sensors: SensorManager? = null
    private var listening = false
    private val jolts = ArrayList<Long>()

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
        main.post { stopRattle() }
    }

    // MARK: Commands

    @Command
    fun play(invoke: Invoke) {
        val args = invoke.parseArgs(PlayArgs::class.java)
        main.post {
            try {
                args.wav?.let { dings[args.tier] = load(args.tier, "ding", it) }
                if (resumed && ringerNormal()) {
                    val id = dings[args.tier] ?: throw IllegalStateException("no sound for tier ${args.tier}; send it first")
                    whenLoaded(id) { p -> p.play(id, 1f, 1f, 1, 0, 1f) }
                }
                invoke.resolve()
            } catch (error: Exception) {
                invoke.reject("jingle: ${error.message}")
            }
        }
    }

    @Command
    fun setShake(invoke: Invoke) {
        val args = invoke.parseArgs(ShakeArgs::class.java)
        main.post {
            try {
                args.wav?.let { rattles[args.tier] = load(args.tier, "rattle", it) }
                val tierChanged = shakeTier != args.tier
                shakeTier = args.tier
                shakeEnabled = args.enabled
                if (args.enabled) {
                    if (tierChanged && rattleStream != 0) { stopRattle(); startRattle() }
                    startMotionIfWanted()
                } else {
                    stopMotion()
                    stopRattle()
                }
                invoke.resolve()
            } catch (error: Exception) {
                invoke.reject("jingle: ${error.message}")
            }
        }
    }

    // MARK: Sounds (main thread)

    private fun pool(): SoundPool = pool ?: SoundPool.Builder()
        .setMaxStreams(3)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build()
        )
        .build()
        .also { p ->
            p.setOnLoadCompleteListener { _, id, status ->
                if (status == 0) {
                    loaded.add(id)
                    pending.remove(id)?.invoke(p)
                }
            }
            pool = p
        }

    private val pending = HashMap<Int, (SoundPool) -> Unit>()

    private fun load(tier: String, kind: String, base64: String): Int {
        val bytes = Base64.decode(base64, Base64.DEFAULT)
        val file = File(activity.cacheDir, "klimper-$kind-$tier.wav")
        file.writeBytes(bytes)
        val map = if (kind == "ding") dings else rattles
        map[tier]?.let { pool().unload(it) }
        return pool().load(file.path, 1)
    }

    /** Run once the sound is decoded (the first play of a sound may come before that). */
    private fun whenLoaded(id: Int, action: (SoundPool) -> Unit) {
        val p = pool()
        if (loaded.contains(id)) action(p) else pending[id] = action
    }

    private fun ringerNormal(): Boolean {
        val audio = activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager
        return audio.ringerMode == AudioManager.RINGER_MODE_NORMAL
    }

    // MARK: Rattle (main thread)

    private fun startRattle() {
        if (rattleStream != 0 || !resumed) return
        val id = rattles[shakeTier] ?: return
        if (!ringerNormal()) { rattleStream = -1; return } // felt, not heard
        whenLoaded(id) { p -> if (rattleStream == -1 || rattleStream == 0) rattleStream = p.play(id, 1f, 1f, 1, -1, 1f) }
        if (rattleStream == 0) rattleStream = -1 // marks "rattling" until the stream id arrives
    }

    private fun stopRattle() {
        stopRattle?.let { main.removeCallbacks(it) }
        stopRattle = null
        val stream = rattleStream
        rattleStream = 0
        if (stream > 0) pool?.stop(stream)
    }

    private fun keepRattling() {
        stopRattle?.let { main.removeCallbacks(it) }
        val r = Runnable { stopRattle() }
        stopRattle = r
        main.postDelayed(r, STILL_MS)
    }

    private fun tap(force: Double) {
        val now = System.currentTimeMillis()
        if (now - lastTap < TAP_SPACING_MS) return
        lastTap = now
        val vibrator = vibrator() ?: return
        if (!vibrator.hasVibrator()) return
        val weight = when (shakeTier) { "pouch" -> 1.0; "handful" -> 0.85; "few" -> 0.7; else -> 0.55 }
        val strength = min(1.0, max(0.45, weight * (0.6 + 0.4 * force)))
        val pulseMs = (18 + 30 * strength).toLong()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            vibrator.vibrate(VibrationEffect.createOneShot(pulseMs, (strength * 255).toInt()))
        } else {
            @Suppress("DEPRECATION")
            vibrator.vibrate(pulseMs)
        }
    }

    private fun vibrator(): Vibrator? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (activity.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as? VibratorManager)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            activity.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
        }

    // MARK: Motion

    private fun startMotionIfWanted() {
        if (!shakeEnabled || listening || !resumed) return
        val manager = sensors ?: (activity.getSystemService(Context.SENSOR_SERVICE) as SensorManager).also { sensors = it }
        // Linear acceleration has gravity removed; fall back to the raw accelerometer.
        val sensor = manager.getDefaultSensor(Sensor.TYPE_LINEAR_ACCELERATION)
            ?: manager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)
            ?: return
        jolts.clear()
        listening = manager.registerListener(this, sensor, SensorManager.SENSOR_DELAY_GAME)
    }

    private fun stopMotion() {
        if (listening) sensors?.unregisterListener(this)
        listening = false
        jolts.clear()
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    // Sensor events arrive on the main thread by default.
    override fun onSensorChanged(event: SensorEvent) {
        val x = event.values[0].toDouble()
        val y = event.values[1].toDouble()
        val z = event.values[2].toDouble()
        var magnitude = sqrt(x * x + y * y + z * z) / SensorManager.GRAVITY_EARTH
        if (event.sensor.type == Sensor.TYPE_ACCELEROMETER) {
            magnitude = abs(magnitude - 1.0) // raw accelerometer: take gravity's 1 g out
        }
        if (magnitude < JOLT_G) return
        val now = event.timestamp / 1_000_000 // ns to ms
        jolts.removeAll { now - it > WINDOW_MS }
        jolts.add(now)
        val force = min(1.0, max(0.0, (magnitude - JOLT_G) / (4.0 - JOLT_G)))
        if (!resumed) return
        if (rattleStream != 0) {
            keepRattling()
            tap(force)
        } else if (jolts.size >= 2) {
            startRattle()
            keepRattling()
            tap(force)
        }
    }
}
