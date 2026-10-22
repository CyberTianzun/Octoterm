package ai.eclosion.octoterm.android.push

import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.update

/** Application-owned SDK registration. Activity changes and caller timeouts do not own the channel. */
class AliyunDeviceRegistration(
    private val start: (onSuccess: (String) -> Unit, onFailure: (Exception) -> Unit) -> Unit,
    private val onDeviceReady: () -> Unit,
    private val readExistingDeviceId: () -> String? = { null },
    private val onExistingRegistrationPending: () -> Unit = {},
) {
    private sealed interface State {
        data object Pending : State
        data class Ready(val deviceId: String) : State
        data class Failed(val error: Exception) : State
    }

    private val state = MutableStateFlow<State>(State.Pending)
    private val started = AtomicBoolean(false)
    private val waitingForExisting = AtomicBoolean(false)
    val awaitingExistingDeviceId: Boolean get() = waitingForExisting.get()

    suspend fun deviceId(): String {
        if (waitingForExisting.get()) {
            val existing = existingDeviceId()
            if (existing != null) acceptExistingDeviceId(existing)
            else onExistingRegistrationPending()
        }
        if (started.compareAndSet(false, true)) {
            state.value = State.Pending
            try {
                start(::succeeded, ::failed)
            } catch (error: Exception) {
                // A synchronous startup failure did not establish a registration to retry internally.
                state.value = State.Failed(error)
                started.set(false)
                throw error
            }
        }
        return when (val result = state.first { it !is State.Pending }) {
            is State.Ready -> result.deviceId
            is State.Failed -> throw result.error
            State.Pending -> error("Registration has not completed")
        }
    }

    private fun existingDeviceId(): String? = runCatching(readExistingDeviceId).getOrNull()?.takeIf { it.isNotBlank() }

    /** Called by the application's observer when an existing SDK attempt produces its ID. */
    fun acceptExistingDeviceId(id: String) {
        if (id.isNotBlank() && waitingForExisting.compareAndSet(true, false)) succeeded(id)
    }

    private fun failed(error: Exception) {
        if (error is AliyunPushException && error.code == "PUSH_20110") {
            waitingForExisting.set(true)
            val existing = existingDeviceId()
            if (existing != null) succeeded(existing)
            else {
                // Keep waiting for a real ID without claiming success or registering again.
                state.update { if (it is State.Ready) it else State.Pending }
                if (state.value !is State.Ready) onExistingRegistrationPending()
                else waitingForExisting.set(false)
            }
            return
        }
        waitingForExisting.set(false)
        state.value = State.Failed(error)
    }

    private fun succeeded(id: String) {
        waitingForExisting.set(false)
        if (id.isBlank()) {
            state.value = State.Failed(AliyunPushException("device-id", "EMPTY_DEVICE_ID"))
            return
        }
        val previous = state.getAndUpdate { State.Ready(id) }
        // Always notify the application, even if the waiting caller is cancelled just as this
        // callback arrives. Cloud registration is idempotent and independent of that caller.
        if (previous !is State.Ready || previous.deviceId != id) onDeviceReady()
    }
}
