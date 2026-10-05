// SPDX-License-Identifier: AGPL-3.0-or-later
// Receives Firebase messages. The gateway sends a data message with no notification block, so
// Android shows nothing by itself: this shows one fixed, generic notification (no amount, no
// address, nothing from the message), and wakes the session loop if the app is running.
package com.clapandpay.klimper.xchonnect

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage

class KlimperMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) {
        KlimperXchonnectPlugin.current?.deviceToken(token)
    }

    override fun onMessageReceived(message: RemoteMessage) {
        KlimperXchonnectPlugin.current?.wake()
        // On screen, the approval appears by itself.
        if (KlimperXchonnectPlugin.foreground) return
        show()
    }

    private fun show() {
        val manager = NotificationManagerCompat.from(this)
        if (!manager.areNotificationsEnabled()) return
        val channelId = "klimper_requests"
        manager.createNotificationChannel(
            NotificationChannel(
                channelId,
                getString(R.string.klimper_push_channel),
                NotificationManager.IMPORTANCE_HIGH
            )
        )
        val open = packageManager.getLaunchIntentForPackage(packageName)?.apply {
            addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            putExtra(KlimperXchonnectPlugin.EXTRA_WAKE, true)
        }
        val tap = open?.let {
            PendingIntent.getActivity(
                this, 0, it, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
            )
        }
        val notification = NotificationCompat.Builder(this, channelId)
            .setSmallIcon(applicationInfo.icon)
            .setContentTitle(getString(R.string.klimper_push_title))
            .setContentText(getString(R.string.klimper_push_body))
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE)
            .setAutoCancel(true)
            .setContentIntent(tap)
            .build()
        // One notification at a time: a second wake replaces the first.
        manager.notify(1, notification)
    }
}
