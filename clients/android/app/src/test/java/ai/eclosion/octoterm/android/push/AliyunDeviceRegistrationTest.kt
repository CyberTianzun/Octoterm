package ai.eclosion.octoterm.android.push

import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.yield
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AliyunDeviceRegistrationTest {
    @Test
    fun repeatedForegroundRefreshesReuseOneSdkRegistration() = runBlocking {
        var calls = 0
        var cloudRefreshes = 0
        val registration = AliyunDeviceRegistration({ ready, _ -> calls++; ready("device") }, { cloudRefreshes++ })
        repeat(3) { assertEquals("device", registration.deviceId()) }
        assertEquals(1, calls)
        assertEquals(1, cloudRefreshes)
    }

    @Test
    fun simultaneousCallersShareTheChannel() = runBlocking {
        var calls = 0
        lateinit var ready: (String) -> Unit
        val registration = AliyunDeviceRegistration({ success, _ -> calls++; ready = success }, {})
        val first = async { registration.deviceId() }
        val second = async { registration.deviceId() }
        yield()
        ready("device")
        assertEquals("device", first.await())
        assertEquals("device", second.await())
        assertEquals(1, calls)
    }

    @Test
    fun sdkRecoveryAfterInitialFailureRefreshesCloudWithoutOpeningAServer() = runBlocking {
        lateinit var ready: (String) -> Unit
        var refreshes = 0
        val registration = AliyunDeviceRegistration({ success, failed ->
            ready = success
            failed(AliyunPushException("register", "PUSH_10109"))
        }, { refreshes++ })
        val failure = runCatching { registration.deviceId() }.exceptionOrNull()
        assertTrue(failure is AliyunPushException)
        ready("device-after-recovery")
        assertEquals(1, refreshes)
        assertEquals("device-after-recovery", registration.deviceId())
        ready("device-after-recovery")
        assertEquals(1, refreshes)
        ready("rotated-device")
        assertEquals(2, refreshes)
        assertEquals("rotated-device", registration.deviceId())
    }

    @Test
    fun callerTimeoutDoesNotDiscardALateSuccessfulRegistration() = runBlocking {
        lateinit var ready: (String) -> Unit
        var refreshes = 0
        val registration = AliyunDeviceRegistration({ success, _ -> ready = success }, { refreshes++ })
        assertNull(withTimeoutOrNull(20) { registration.deviceId() })
        ready("late-device")
        assertEquals(1, refreshes)
        assertEquals("late-device", registration.deviceId())
    }

    @Test
    fun synchronousInitializationFailureCanBeRetried() = runBlocking {
        var calls = 0
        val registration = AliyunDeviceRegistration({ ready, _ ->
            calls++
            if (calls == 1) throw IllegalStateException("not initialized")
            ready("device")
        }, {})
        assertTrue(runCatching { registration.deviceId() }.exceptionOrNull() is IllegalStateException)
        assertEquals("device", registration.deviceId())
        assertEquals(2, calls)
    }

    @Test
    fun duplicateSdkRegistrationReusesAnExistingDeviceId() = runBlocking {
        var starts = 0
        var refreshes = 0
        val registration = AliyunDeviceRegistration(
            start = { _, failed -> starts++; failed(AliyunPushException("register", "PUSH_20110")) },
            onDeviceReady = { refreshes++ },
            readExistingDeviceId = { "existing-device" },
        )
        repeat(3) { assertEquals("existing-device", registration.deviceId()) }
        assertEquals(1, starts)
        assertEquals(1, refreshes)
    }

    @Test
    fun duplicateWithoutADeviceIdWaitsInsteadOfFalselyReportingSuccessOrFailure() = runBlocking {
        var starts = 0
        var refreshes = 0
        var observations = 0
        val registration = AliyunDeviceRegistration(
            start = { _, failed -> starts++; failed(AliyunPushException("register", "PUSH_20110")) },
            onDeviceReady = { refreshes++ },
            readExistingDeviceId = { null },
            onExistingRegistrationPending = { observations++ },
        )
        assertNull(withTimeoutOrNull(20) { registration.deviceId() })
        assertEquals(0, refreshes)
        assertEquals(1, observations)
        registration.acceptExistingDeviceId("")
        assertEquals(0, refreshes)
        registration.acceptExistingDeviceId("eventually-registered-device")
        assertEquals(1, refreshes)
        assertEquals("eventually-registered-device", registration.deviceId())
        assertEquals(1, starts)
    }

    @Test
    fun aReadySdkDoesNotLoseItsDeviceIdBecauseOfADuplicateCallback() = runBlocking {
        lateinit var failed: (Exception) -> Unit
        var starts = 0
        var refreshes = 0
        val registration = AliyunDeviceRegistration({ ready, failure ->
            starts++; failed = failure; ready("device")
        }, { refreshes++ })
        assertEquals("device", registration.deviceId())
        failed(AliyunPushException("register", "PUSH_20110"))
        assertEquals("device", registration.deviceId())
        assertEquals(1, starts)
        assertEquals(1, refreshes)
    }

    @Test
    fun genuineSdkFailuresAreNotMaskedByACachedDeviceId() = runBlocking {
        var starts = 0
        val registration = AliyunDeviceRegistration(
            start = { _, failed -> starts++; failed(AliyunPushException("register", "PUSH_10103")) },
            onDeviceReady = {}, readExistingDeviceId = { "stale-device" },
        )
        repeat(2) {
            val error = runCatching { registration.deviceId() }.exceptionOrNull() as AliyunPushException
            assertEquals("PUSH_10103", error.code)
        }
        assertEquals(1, starts)
    }

    @Test
    fun anObserverCannotReplaceARealFailureWithAStaleDeviceId() = runBlocking {
        lateinit var failed: (Exception) -> Unit
        val registration = AliyunDeviceRegistration({ _, failure ->
            failed = failure
            failure(AliyunPushException("register", "PUSH_20110"))
        }, {})
        assertNull(withTimeoutOrNull(20) { registration.deviceId() })
        failed(AliyunPushException("register", "PUSH_10103"))
        registration.acceptExistingDeviceId("stale-device")
        val error = runCatching { registration.deviceId() }.exceptionOrNull() as AliyunPushException
        assertEquals("PUSH_10103", error.code)
    }
}
