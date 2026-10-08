// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on Android (clapandpay TASK-111), the same as the Swift: `play` rings the
// tier's ding once; while the shake is armed, every turn of the hand is a few coins landing, each
// a single coin hit (one of eight, at its own pitch and loudness) and, where the hand can tell
// them apart, a vibration pulse. Count, choice, order, spacing, pitch and loudness are drawn fresh
// every turn; the tier sets the ranges. Sound is skipped when the ringer is silent or vibrate (the
// hand still feels it). Nothing runs unless the app is resumed.
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
import android.os.SystemClock
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.view.HapticFeedbackConstants
import android.util.Base64
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin
import java.io.File
import kotlin.math.ln
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.math.sqrt
import kotlin.random.Random

@InvokeArg
class PlayArgs {
    lateinit var tier: String
    var wav: String? = null
}

@InvokeArg
class FeelArgs {
    lateinit var kind: String
}

@InvokeArg
class ShakeArgs {
    var enabled: Boolean = false
    lateinit var tier: String
    var grains: List<String>? = null
}

private class Tier(val coins: IntRange, val weight: Double, val rate: ClosedFloatingPointRange<Double>, val loud: Double)

@TauriPlugin
class KlimperJinglePlugin(private val activity: Activity) : Plugin(activity), SensorEventListener {
    companion object {
        private val TIERS = mapOf(
            "coin" to Tier(1..1, 0.65, 1.0..1.1, 0.6),
            "few" to Tier(2..4, 0.8, 0.94..1.08, 0.7),
            "handful" to Tier(3..6, 0.9, 0.92..1.06, 0.85),
            "pouch" to Tier(5..8, 1.0, 0.9..1.04, 1.0),
        )
        private const val JOLT_G = 1.3 // a turn of the hand
        private const val CALM_G = 0.6 // the hand has turned; the next jolt counts
        private const val TURN_MS = 300L
    }

    private val main = Handler(Looper.getMainLooper())
    private val pool = SoundPool.Builder()
        .setMaxStreams(8)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build()
        )
        .build()
    private val ready = HashSet<Int>()
    private var pendingDing: Int? = null
    private val dings = HashMap<String, Int>()
    private var grains = listOf<Int>()
    private var tier = "coin"
    private var armed = false
    private var resumed = true
    private var turned = true
    private var lastTurn = 0L
    private var lastFirst = -1

    init {
        // SoundPool loads asynchronously: a sound plays only once it is ready.
        pool.setOnLoadCompleteListener { p, id, status ->
            if (status != 0) return@setOnLoadCompleteListener
            ready.add(id)
            if (id == pendingDing) {
                pendingDing = null
                if (resumed && soundOn()) p.play(id, 1f, 1f, 1, 0, 1f)
            }
        }
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        watch()
    }

    override fun onPause() {
        super.onPause()
        resumed = false
        rest()
    }

    override fun onDestroy() {
        rest()
        pool.release()
        super.onDestroy()
    }

    @Command
    fun play(invoke: Invoke) {
        val args = invoke.parseArgs(PlayArgs::class.java)
        args.wav?.let { dings[args.tier] = load("ding-${args.tier}", it) }
        val id = dings[args.tier]
        if (id != null && resumed && soundOn()) {
            if (id in ready) pool.play(id, 1f, 1f, 1, 0, 1f) else pendingDing = id
        }
        invoke.resolve()
    }

    @Command
    fun setShake(invoke: Invoke) {
        val args = invoke.parseArgs(ShakeArgs::class.java)
        args.grains?.let { wavs -> grains = wavs.mapIndexed { i, w -> load("grain-${i + 1}", w) } }
        tier = args.tier
        armed = args.enabled
        if (armed) watch() else rest()
        invoke.resolve()
    }

    /** A control's system haptic (the slide to approve); the phone's haptics setting decides. */
    @Command
    fun feel(invoke: Invoke) {
        val kind = invoke.parseArgs(FeelArgs::class.java).kind
        val constant = when (kind) {
            "grab" -> HapticFeedbackConstants.VIRTUAL_KEY
            "tick" -> HapticFeedbackConstants.CLOCK_TICK
            "done" -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) HapticFeedbackConstants.CONFIRM
                else HapticFeedbackConstants.LONG_PRESS
            else -> null
        }
        if (constant != null) activity.runOnUiThread { activity.window.decorView.performHapticFeedback(constant) }
        invoke.resolve()
    }

    private fun load(name: String, base64: String): Int {
        val file = File(activity.cacheDir, "klimper-$name.wav")
        file.writeBytes(Base64.decode(base64, Base64.DEFAULT))
        return pool.load(file.path, 1)
    }

    private fun sensors() = activity.getSystemService(Context.SENSOR_SERVICE) as SensorManager

    private fun watch() {
        if (!armed || !resumed || grains.isEmpty()) return
        val sensor = sensors().getDefaultSensor(Sensor.TYPE_LINEAR_ACCELERATION) ?: return
        turned = true
        sensors().registerListener(this, sensor, SensorManager.SENSOR_DELAY_GAME)
    }

    private fun rest() {
        sensors().unregisterListener(this)
        main.removeCallbacksAndMessages(null)
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    override fun onSensorChanged(event: SensorEvent) {
        val (x, y, z) = event.values
        val g = sqrt((x * x + y * y + z * z).toDouble()) / SensorManager.GRAVITY_EARTH
        if (g < CALM_G) turned = true
        val now = SystemClock.elapsedRealtime()
        if (!turned || g < JOLT_G || now - lastTurn < TURN_MS || !resumed) return
        turned = false
        lastTurn = now
        turnOfTheHand(min(1.0, (g - JOLT_G) / 2.5))
    }

    // A few coins landing in the hand: each a coin hit, and a pulse where the hand can tell them apart.
    private fun turnOfTheHand(force: Double) {
        val t = TIERS[tier] ?: return
        val order = grains.indices.shuffled().toMutableList()
        if (order.size > 1 && order[0] == lastFirst) order.add(order.removeAt(0))
        lastFirst = order.firstOrNull() ?: return
        var coins = min(t.coins.random(), order.size)
        if (coins == 1 && order.size > 1 && Random.nextDouble() < 0.3) coins = 2
        val base = max(0.6, t.loud * (0.6 + 0.4 * force))
        val sound = soundOn()
        val timings = ArrayList<Long>()
        val amplitudes = ArrayList<Int>()
        var at = 0.0
        var lastPulse = 0.0
        var pulses = 0
        for (k in 0 until coins) {
            if (k > 0) at += min(0.06, max(0.008, -ln(Random.nextDouble(0.05, 1.0)) * 0.022 * (0.8 + 0.6 * force)))
            val id = grains[order[k]]
            val volume = (if (k == 0) base else base * Random.nextDouble(0.2, 0.7) * 0.85.pow(k)).toFloat()
            val rate = Random.nextDouble(t.rate.start, t.rate.endInclusive).toFloat()
            if (sound && id in ready) main.postDelayed({ pool.play(id, volume, volume, 1, 0, rate) }, (at * 1000).toLong())
            if (k > 0 && (at - lastPulse < 0.045 || pulses >= 4)) continue
            val strength = if (k == 0) t.weight * (0.8 + 0.2 * force)
            else max(0.35, t.weight * 0.82.pow(pulses) * Random.nextDouble(0.88, 1.12))
            timings.add(if (k == 0) 0L else ((at - lastPulse) * 1000).toLong() - 20L); amplitudes.add(0)
            timings.add(20L); amplitudes.add((min(1.0, strength) * 255).toInt())
            lastPulse = at
            pulses++
        }
        val vibrator = vibrator() ?: return
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && vibrator.hasVibrator()) {
            vibrator.vibrate(VibrationEffect.createWaveform(timings.toLongArray(), amplitudes.toIntArray(), -1))
        }
    }

    private fun soundOn() =
        (activity.getSystemService(Context.AUDIO_SERVICE) as AudioManager).ringerMode == AudioManager.RINGER_MODE_NORMAL

    private fun vibrator(): Vibrator? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            (activity.getSystemService(Context.VIBRATOR_MANAGER_SERVICE) as? VibratorManager)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            activity.getSystemService(Context.VIBRATOR_SERVICE) as? Vibrator
        }
}
