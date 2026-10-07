// SPDX-License-Identifier: AGPL-3.0-or-later
// The Klimper sound on Android (clapandpay TASK-111), the same as the Swift: `play` rings the
// tier's ding once; `setShake` arms the accelerometer, and every turn of the hand (a jolt, at most
// one every TURN_MS) is one knock and one vibration together. Heavy wallets pulse twice, a thud
// and a clack; a near-empty one ticks once. Sound is skipped when the ringer is silent or vibrate
// (the hand still feels it). Nothing plays or vibrates unless the app is resumed.
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
    var knocks: List<String>? = null
}

@TauriPlugin
class KlimperJinglePlugin(private val activity: Activity) : Plugin(activity), SensorEventListener {
    companion object {
        private const val JOLT_G = 1.3 // acceleration, gravity removed, that counts as a turn
        private const val TURN_MS = 280L // slower than this is a shake, faster is a buzz
    }

    private val pool = SoundPool.Builder()
        .setMaxStreams(3)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build()
        )
        .build()
    private val dings = HashMap<String, Int>()
    private val knocks = HashMap<String, List<Int>>()
    private var tier = "coin"
    private var shaking = false
    private var lastTurn = 0L
    private var resumed = true

    override fun onResume() {
        super.onResume()
        resumed = true
        watch()
    }

    override fun onPause() {
        super.onPause()
        resumed = false
        sensors().unregisterListener(this)
    }

    @Command
    fun play(invoke: Invoke) {
        val args = invoke.parseArgs(PlayArgs::class.java)
        args.wav?.let { dings[args.tier] = load("ding-${args.tier}", it) }
        val id = dings[args.tier]
        if (id != null && resumed && soundOn()) pool.play(id, 1f, 1f, 1, 0, 1f)
        invoke.resolve()
    }

    @Command
    fun setShake(invoke: Invoke) {
        val args = invoke.parseArgs(ShakeArgs::class.java)
        args.knocks?.let { wavs -> knocks[args.tier] = wavs.mapIndexed { i, w -> load("knock-${args.tier}-$i", w) } }
        tier = args.tier
        shaking = args.enabled
        if (shaking) watch() else sensors().unregisterListener(this)
        invoke.resolve()
    }

    private fun load(name: String, base64: String): Int {
        val file = File(activity.cacheDir, "klimper-$name.wav")
        file.writeBytes(Base64.decode(base64, Base64.DEFAULT))
        return pool.load(file.path, 1)
    }

    private fun sensors() = activity.getSystemService(Context.SENSOR_SERVICE) as SensorManager

    private fun watch() {
        if (!shaking || !resumed) return
        val sensor = sensors().getDefaultSensor(Sensor.TYPE_LINEAR_ACCELERATION) ?: return
        sensors().registerListener(this, sensor, SensorManager.SENSOR_DELAY_GAME)
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    override fun onSensorChanged(event: SensorEvent) {
        val (x, y, z) = event.values
        val g = sqrt((x * x + y * y + z * z).toDouble()) / SensorManager.GRAVITY_EARTH
        val now = System.currentTimeMillis()
        if (g < JOLT_G || now - lastTurn < TURN_MS || !resumed) return
        lastTurn = now
        turnOfTheHand(min(1.0, (g - JOLT_G) / 2.5))
    }

    // One knock and one vibration, together.
    private fun turnOfTheHand(force: Double) {
        knocks[tier]?.randomOrNull()?.let { id ->
            val volume = (0.6 + 0.4 * force).toFloat()
            if (soundOn()) pool.play(id, volume, volume, 1, 0, (0.9 + 0.15 * Math.random()).toFloat())
        }
        val vibrator = vibrator() ?: return
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val effect = when (tier) {
            "coin" -> VibrationEffect.createOneShot(25, 180)
            "few" -> VibrationEffect.createOneShot(40, 220)
            else -> VibrationEffect.createWaveform(longArrayOf(0, 45, 35, 30), intArrayOf(0, 255, 0, if (tier == "pouch") 230 else 150), -1)
        }
        vibrator.vibrate(effect)
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
