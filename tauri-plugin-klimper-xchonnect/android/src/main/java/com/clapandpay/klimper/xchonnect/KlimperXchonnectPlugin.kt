// SPDX-License-Identifier: AGPL-3.0-or-later
// Push wake-ups for Xchonnect sessions on Android (clapandpay TASK-087), the counterpart of
// ios/Sources/KlimperXchonnectPlugin.swift. It asks for the notification permission,
// fetches the Firebase token and reports only two things to Rust over a channel: the token,
// and "a notification arrived". The message itself is never read.
package com.clapandpay.klimper.xchonnect

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import android.webkit.WebView
import androidx.core.content.ContextCompat
import app.tauri.PermissionState
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.Permission
import app.tauri.annotation.PermissionCallback
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.firebase.messaging.FirebaseMessaging

private const val TAG = "KlimperPush"
private const val ALIAS = "notifications"

@InvokeArg
class StartArgs {
    lateinit var channel: Channel
}

@TauriPlugin(
    permissions = [
        Permission(strings = [Manifest.permission.POST_NOTIFICATIONS], alias = ALIAS)
    ]
)
class KlimperXchonnectPlugin(private val activity: Activity) : Plugin(activity) {
    companion object {
        const val EXTRA_WAKE = "klimper_push_wake"

        @Volatile
        var current: KlimperXchonnectPlugin? = null

        /** The app is on screen: the session loop is already reading the mailbox. */
        @Volatile
        var foreground = false
    }

    private var channel: Channel? = null

    // A tap can start the app before Rust has handed us a channel. One flag is enough: a
    // wake-up carries nothing else.
    private var wakeBeforeChannel = false

    override fun load(webView: WebView) {
        super.load(webView)
        current = this
        if (activity.intent?.getBooleanExtra(EXTRA_WAKE, false) == true) wake()
    }

    override fun onNewIntent(intent: Intent) {
        if (intent.getBooleanExtra(EXTRA_WAKE, false)) wake()
    }

    override fun onResume() {
        foreground = true
    }

    override fun onPause() {
        foreground = false
    }

    /**
     * Called when the app starts. Fetches the token silently when notifications are already
     * allowed; it never shows the permission prompt (that waits for the first pairing).
     */
    @Command
    fun start(invoke: Invoke) {
        channel = invoke.parseArgs(StartArgs::class.java).channel
        if (wakeBeforeChannel) {
            wakeBeforeChannel = false
            wake()
        }
        val state = state()
        if (state == "granted") fetchToken()
        invoke.resolve(JSObject().put("state", state))
    }

    /** The first pairing asks. Android shows its prompt (13 and later); after a refusal this only reports it. */
    @Command
    fun requestPermission(invoke: Invoke) {
        if (state() != "undetermined" || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            finish(invoke)
            return
        }
        requestPermissionForAlias(ALIAS, invoke, "permissionResult")
    }

    @PermissionCallback
    fun permissionResult(invoke: Invoke) = finish(invoke)

    private fun finish(invoke: Invoke) {
        val state = state()
        if (state == "granted") fetchToken()
        invoke.resolve(JSObject().put("state", state))
    }

    private fun state(): String {
        // Before Android 13 there is no runtime permission: notifications are on unless the
        // user switched them off in settings.
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return "granted"
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return "granted"
        }
        return when (getPermissionState(ALIAS)) {
            PermissionState.DENIED -> "denied"
            else -> "undetermined"
        }
    }

    private fun fetchToken() {
        try {
            FirebaseMessaging.getInstance().token.addOnCompleteListener { task ->
                if (task.isSuccessful) {
                    deviceToken(task.result)
                } else {
                    // No google-services.json, or no Play services: polling carries on.
                    Log.w(TAG, "no Firebase token: ${task.exception?.message}")
                }
            }
        } catch (error: IllegalStateException) {
            Log.w(TAG, "Firebase is not configured: ${error.message}")
        }
    }

    /** Also called by [KlimperMessagingService] when Firebase issues a new token. */
    fun deviceToken(token: String) {
        channel?.send(
            JSObject().put("kind", "token").put("platform", "fcm").put("token", token)
        )
    }

    /** A wake-up arrived. Nothing it carried is looked at. */
    fun wake() {
        val open = channel
        if (open == null) {
            wakeBeforeChannel = true
            return
        }
        open.send(JSObject().put("kind", "wake"))
    }
}
