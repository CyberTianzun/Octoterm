package ai.eclosion.octoterm.android.push

import android.content.Context
import android.util.Log
import com.alibaba.sdk.android.push.MessageReceiver
import com.alibaba.sdk.android.push.notification.CPushMessage

class AliyunMessageReceiver : MessageReceiver() {
    override fun onMessage(context: Context, message: CPushMessage) {
        Log.i("OctotermPush", "Aliyun data message received")
        if (!PushRegistrar.acceptsMessage(context, "aliyun")) {
            Log.i("OctotermPush", "Message ignored: Cloud account or push route inactive")
            return
        }
        val push = AgentPush.parseMessage(message.content ?: return) ?: return
        AgentNotifications.handle(context.applicationContext, push)
    }

    // Cloud uses data messages, leaving localization, expiry and deep links to AgentNotifications.
    override fun onNotification(context: Context, title: String?, summary: String?, extra: MutableMap<String, String>?) = Unit
    override fun onNotificationOpened(context: Context, title: String?, summary: String?, extra: MutableMap<String, String>?) = Unit
    override fun onNotificationRemoved(context: Context, messageId: String?) = Unit
}
