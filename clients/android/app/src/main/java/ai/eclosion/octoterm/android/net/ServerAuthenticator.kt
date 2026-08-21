package ai.eclosion.octoterm.android.net

import ai.eclosion.octoterm.android.wire.ControlJson
import ai.eclosion.octoterm.android.wire.Frame
import ai.eclosion.octoterm.android.wire.ServerMsg
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.resume

sealed class AuthException(message: String, cause: Throwable? = null) : Exception(message, cause) {
    class Rejected(val serverMessage: String?) : AuthException(serverMessage.orEmpty())
    class Unreachable(cause: Throwable?) : AuthException(cause?.message.orEmpty(), cause)
    data object Timeout : AuthException("timeout")
    data object Closed : AuthException("closed")
    class Protocol(detail: String) : AuthException(detail)
}

class ServerAuthenticator(
    private val client: OkHttpClient = defaultClient(),
) {
    suspend fun authenticate(webSocketUrl: String, token: String): Result<Unit> {
        val outcome = withTimeoutOrNull(TIMEOUT_MS) {
            suspendCancellableCoroutine { cont ->
                val done = AtomicBoolean(false)
                fun complete(result: Result<Unit>) {
                    if (done.compareAndSet(false, true) && cont.isActive) {
                        cont.resume(result)
                    }
                }

                val request = Request.Builder().url(webSocketUrl).build()
                val socket = client.newWebSocket(
                    request,
                    object : WebSocketListener() {
                        override fun onOpen(webSocket: WebSocket, response: Response) {
                            val sent = webSocket.send(ByteString.of(*ControlJson.encodeHello(token)))
                            if (!sent) {
                                webSocket.cancel()
                                complete(Result.failure(AuthException.Unreachable(null)))
                            }
                        }

                        override fun onMessage(webSocket: WebSocket, text: String) {
                            webSocket.cancel()
                            complete(Result.failure(AuthException.Protocol("text")))
                        }

                        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                            val frame = try {
                                Frame.decode(bytes.toByteArray())
                            } catch (_: Exception) {
                                webSocket.cancel()
                                complete(Result.failure(AuthException.Protocol("frame")))
                                return
                            }
                            if (frame.channel != Frame.CONTROL_CHANNEL) {
                                return
                            }
                            when (val msg = try {
                                ControlJson.parse(frame.payload)
                            } catch (_: Exception) {
                                webSocket.cancel()
                                complete(Result.failure(AuthException.Protocol("json")))
                                return
                            }) {
                                is ServerMsg.HelloOk -> {
                                    webSocket.close(1000, null)
                                    complete(Result.success(Unit))
                                }
                                is ServerMsg.Error -> {
                                    webSocket.cancel()
                                    complete(Result.failure(AuthException.Rejected(msg.message)))
                                }
                                else -> {
                                    webSocket.cancel()
                                    complete(Result.failure(AuthException.Protocol(msg::class.java.simpleName)))
                                }
                            }
                        }

                        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                            complete(Result.failure(AuthException.Unreachable(t)))
                        }

                        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                            complete(Result.failure(AuthException.Closed))
                        }
                    },
                )
                cont.invokeOnCancellation { socket.cancel() }
            }
        }
        return outcome ?: Result.failure(AuthException.Timeout)
    }

    companion object {
        private const val TIMEOUT_MS = 12_000L

        private fun defaultClient(): OkHttpClient {
            return OkHttpClient.Builder()
                .connectTimeout(8, TimeUnit.SECONDS)
                .readTimeout(15, TimeUnit.SECONDS)
                .retryOnConnectionFailure(false)
                .build()
        }
    }
}
