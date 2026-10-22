package ai.eclosion.octoterm.android.push

import ai.eclosion.octoterm.android.BuildConfig
import android.app.Application
import android.util.Log
import com.alibaba.sdk.android.push.CloudPushService
import com.alibaba.sdk.android.push.CommonCallback
import com.alibaba.sdk.android.push.noonesdk.PushInitConfig
import com.alibaba.sdk.android.push.noonesdk.PushServiceFactory
import java.io.File
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

object AliyunPush {
    val configured: Boolean get() = BuildConfig.ALIYUN_APP_KEY.isNotBlank() && BuildConfig.ALIYUN_APP_SECRET.isNotBlank()
    private var initialized = false
    private var initializationFailure = "NOT_INITIALIZED"
    private var registration: AliyunDeviceRegistration? = null
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val observingExistingRegistration = AtomicBoolean(false)

    /** Parameter initialization only; called on the Application main thread in every SDK process. */
    fun initialize(app: Application) {
        if (!configured || initialized) return
        try {
            PushServiceFactory.init(PushInitConfig.Builder().application(app)
                .appKey(BuildConfig.ALIYUN_APP_KEY).appSecret(BuildConfig.ALIYUN_APP_SECRET).build())
            PushServiceFactory.getCloudPushService().setLogLevel(CloudPushService.LOG_OFF)
            val service = PushServiceFactory.getCloudPushService()
            registration = AliyunDeviceRegistration(
                start = { succeeded, failed ->
                    service.register(app, object : CommonCallback {
                        override fun onSuccess(result: String?) {
                            Log.i("OctotermPush", "Aliyun SDK registration ready")
                            succeeded(service.deviceId.orEmpty())
                        }
                        override fun onFailed(code: String?, message: String?) {
                            val failure = AliyunPushException("register", code)
                            if (failure.code == "PUSH_20110") Log.i("OctotermPush", "Reusing existing Aliyun SDK registration")
                            else Log.w("OctotermPush", "Aliyun SDK registration failed: ${failure.code}")
                            failed(failure)
                        }
                    })
                },
                onDeviceReady = { PushRegistrar.refreshOnStart(app) },
                readExistingDeviceId = { service.deviceId },
                onExistingRegistrationPending = {
                    if (observingExistingRegistration.compareAndSet(false, true)) {
                        // A duplicate call may not receive the original attempt's success callback.
                        // Observe that attempt without restarting it or tying it to a screen.
                        scope.launch {
                            try {
                                var interval = 500L
                                while (isActive && File(app.filesDir, "cloud.json").isFile &&
                                    registration?.awaitingExistingDeviceId == true) {
                                    val id = runCatching { service.deviceId }.getOrNull()
                                    if (!id.isNullOrBlank()) {
                                        registration?.acceptExistingDeviceId(id)
                                        return@launch
                                    }
                                    delay(interval)
                                    interval = (interval * 2).coerceAtMost(10_000L)
                                }
                            } finally { observingExistingRegistration.set(false) }
                        }
                    }
                },
            )
            initialized = true
        } catch (error: Exception) {
            initializationFailure = error.javaClass.simpleName
            initialized = false
        }
    }

    /** Establish the channel only after Cloud has selected Aliyun for the signed-in device. */
    suspend fun deviceId(app: Application): String = withContext(Dispatchers.Main.immediate) {
        if (!initialized) initialize(app)
        if (!initialized) throw AliyunPushException("initialize", initializationFailure)
        checkNotNull(registration).deviceId()
    }
}
