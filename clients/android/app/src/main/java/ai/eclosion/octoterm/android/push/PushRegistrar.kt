package ai.eclosion.octoterm.android.push

import android.content.Context
import android.app.Application
import ai.eclosion.octoterm.android.BuildConfig
import android.util.Log
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailabilityLight
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import uniffi.octoterm_cloud_ffi.CloudAccount
import java.io.File
import java.io.IOException
import uniffi.octoterm_cloud_ffi.CloudFfiException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** 取 FCM token 并登记到 Cloud。没有 google-services.json 的构建里整体降级为空操作。 */
object PushRegistrar {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val registration = Mutex()
    private val _state = MutableStateFlow(PushRegistration.Idle)
    val state = _state.asStateFlow()
    private val _failureCode = MutableStateFlow<String?>(null)
    val failureCode = _failureCode.asStateFlow()

    fun pushAvailable(context: Context): Boolean = AliyunPush.configured || FirebaseApp.getApps(context).isNotEmpty()

    fun acceptsMessage(context: Context, provider: String): Boolean = accountFile(context).isFile &&
        context.getSharedPreferences("push", Context.MODE_PRIVATE).getString("provider", null) == provider

    private fun accountFile(context: Context) = File(context.filesDir, "cloud.json")

    fun account(context: Context): CloudAccount =
        CloudAccount(accountFile(context).absolutePath, android.os.Build.MODEL ?: "Android")

    suspend fun registerCurrentToken(context: Context, account: CloudAccount, refreshedToken: String? = null) = registration.withLock {
        if (!pushAvailable(context)) {
            _state.value = PushRegistration.MissingConfiguration
            return@withLock
        }
        _state.value = PushRegistration.Registering
        _failureCode.value = null
        try {
            val config = account.pushConfig()
            val provider = config.platform
            val token = when (provider) {
                "aliyun" -> {
                    if (!AliyunPush.configured) {
                        _state.value = PushRegistration.MissingConfiguration
                        return@withLock
                    }
                    if (config.appKey != BuildConfig.ALIYUN_APP_KEY) {
                        _state.value = PushRegistration.ProviderMismatch
                        return@withLock
                    }
                    AliyunPush.deviceId(context.applicationContext as Application)
                }
                "fcm" -> {
                    if (FirebaseApp.getApps(context).isEmpty()) {
                        _state.value = PushRegistration.MissingConfiguration
                        return@withLock
                    }
                    if (GoogleApiAvailabilityLight.getInstance().isGooglePlayServicesAvailable(context) != ConnectionResult.SUCCESS) {
                        _state.value = PushRegistration.MissingPlayServices
                        return@withLock
                    }
                    refreshedToken ?: suspendCancellableCoroutine { cont ->
                        FirebaseMessaging.getInstance().token
                            .addOnSuccessListener { cont.resume(it) }
                            .addOnFailureListener { cont.resumeWithException(it) }
                    }
                }
                "none" -> { _state.value = PushRegistration.Disabled; return@withLock }
                else -> { _state.value = PushRegistration.UnsupportedProvider; return@withLock }
            }
            context.getSharedPreferences("push", Context.MODE_PRIVATE).edit().putString("provider", provider).apply()
            account.registerPushTarget(provider, token)
            Log.i("OctotermPush", "Cloud push registration refreshed: $provider")
            _state.value = if (provider == "aliyun") PushRegistration.RegisteredAliyun else PushRegistration.Registered
        } catch (e: Exception) {
            _failureCode.value = (e as? AliyunPushException)?.code
            _state.value = when (e) {
                is TimeoutCancellationException -> PushRegistration.TimedOut
                is CancellationException -> PushRegistration.Idle
                is CloudFfiException -> PushRegistration.CloudError
                is IOException -> PushRegistration.NetworkError
                else -> PushRegistration.Failed
            }
            throw e
        }
    }

    /** FCM 换了 token:已登录就重新登记;失败只记日志,下次启动/登录会再登记。 */
    fun onNewToken(context: Context, token: String) {
        val app = context.applicationContext
        if (!accountFile(app).exists()) return
        scope.launch {
            account(app).use { account ->
                if (account.status() == null) return@launch
                registerWithTimeout { registerCurrentToken(app, account, token) }?.let(::logFailure)
            }
        }
    }

    /** App 启动时调用:已登录则补登一次(token 可能在 App 未运行时变过)。 */
    fun refreshOnStart(context: Context) {
        val app = context.applicationContext
        // 从没登录过就没有账号文件:直接返回,不为不用 Cloud 的用户加载原生库。
        if (!accountFile(app).exists()) return
        scope.launch {
            account(app).use { account ->
                if (account.status() == null) return@launch
                registerWithTimeout { registerCurrentToken(app, account) }?.let(::logFailure)
            }
        }
    }

    /** 只记异常类型:FFI 的 reason 文本可能带服务器返回的内容,不进 logcat。 */
    fun logFailure(e: Throwable) {
        if (e is AliyunPushException) {
            Log.w("Octoterm", "Aliyun push failure: stage=${e.stage}, code=${e.code}")
            return
        }
        Log.w("Octoterm", "登记推送 token 失败: ${e::class.simpleName}")
    }
}
