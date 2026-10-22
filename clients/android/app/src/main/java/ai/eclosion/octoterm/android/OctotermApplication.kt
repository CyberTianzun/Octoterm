package ai.eclosion.octoterm.android

import android.app.Application
import ai.eclosion.octoterm.android.i18n.LocaleStore
import ai.eclosion.octoterm.android.push.PushRegistrar
import ai.eclosion.octoterm.android.push.AliyunPush
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging

class OctotermApplication : Application() {
    val connections by lazy { ai.eclosion.octoterm.android.ui.connections.ConnectionController(this) }
    override fun onCreate() {
        super.onCreate()
        AliyunPush.initialize(this)
        // FCM is started explicitly only when the deployment selects it.
        if (FirebaseApp.getApps(this).isNotEmpty()) FirebaseMessaging.getInstance().isAutoInitEnabled = false
        LocaleStore.apply(LocaleStore.load(this))
        PushRegistrar.refreshOnStart(this)
    }
}
