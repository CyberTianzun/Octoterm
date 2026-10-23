package ai.eclosion.octoterm.android.connection

import ai.eclosion.octoterm.android.MainActivity
import ai.eclosion.octoterm.android.OctotermApplication
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.agent.AgentState
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.distinctUntilChanged

/** Keeps the application-owned server transports running independently of the activity. */
class ConnectionService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val controller get() = (application as OctotermApplication).connections
    private var observer: Job? = null

    override fun onCreate() {
        super.onCreate()
        requested.set(true)
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.connections_channel), NotificationManager.IMPORTANCE_LOW)
                .apply { setShowBadge(false) },
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == STOP_ALL) {
            controller.disconnectAll()
            stopSelf()
            return START_NOT_STICKY
        }
        ServiceCompat.startForeground(this, NOTIFICATION, notification(),
            if (Build.VERSION.SDK_INT >= 34) ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING else 0)
        controller.onConnectionServiceRunning(true)
        controller.restoreDesiredConnections()
        observer?.cancel()
        observer = scope.launch {
            controller.state.map {
                Triple(it.keepConnectionsInBackground, it.connectionStatuses, it.connectedAgentCounts)
            }.distinctUntilChanged().collect { (keep, statuses, _) ->
                if (!keep || statuses.values.none { it.active }) {
                    stopSelf()
                } else {
                    getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification())
                }
            }
        }
        return START_STICKY
    }

    private fun notification(): Notification {
        val state = controller.state.value
        val values = state.connectionStatuses.values
        val connected = values.count { it == ServerConnectionStatus.Connected }
        val connecting = values.count { it == ServerConnectionStatus.Connecting || it == ServerConnectionStatus.Reconnecting }
        val connections = getString(R.string.connections_notification_summary, connected, connecting)
        val activity = listOf(AgentState.Working, AgentState.Thinking, AgentState.Waiting, AgentState.Error, AgentState.Idle)
            .mapNotNull { status ->
                val count = state.connectedAgentCounts[status] ?: 0
                if (count <= 0) return@mapNotNull null
                val label = when (status) {
                    AgentState.Working -> R.string.agent_state_working
                    AgentState.Thinking -> R.string.agent_state_thinking
                    AgentState.Waiting -> R.string.agent_state_waiting
                    AgentState.Error -> R.string.agent_state_error
                    AgentState.Idle -> R.string.agent_state_idle
                    AgentState.Done -> return@mapNotNull null
                }
                getString(R.string.connections_notification_agent_count, getString(label), count)
            }.joinToString(" · ")
        val agents = activity.takeIf { it.isNotEmpty() }?.let { getString(R.string.connections_notification_agents, it) }
        val summary = if (agents == null) connections else "$agents · $connections"
        val open = PendingIntent.getActivity(this, 101,
            Intent(this, MainActivity::class.java).setAction(OPEN_CONNECTIONS)
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 102, Intent(this, ConnectionService::class.java).setAction(STOP_ALL),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        return NotificationCompat.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(getString(R.string.connections_notification_title))
            .setContentText(summary)
            .setStyle(NotificationCompat.BigTextStyle().bigText(if (agents == null) connections else "$connections\n$agents"))
            .setContentIntent(open).setOngoing(true).setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE).setPriority(NotificationCompat.PRIORITY_LOW)
            .addAction(0, getString(R.string.connections_disconnect_all), stop).build()
    }

    override fun onDestroy() {
        scope.cancel()
        requested.set(false)
        controller.onConnectionServiceRunning(false)
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val OPEN_CONNECTIONS = "ai.eclosion.octoterm.android.OPEN_CONNECTIONS"
        private const val STOP_ALL = "ai.eclosion.octoterm.android.STOP_CONNECTIONS"
        private const val CHANNEL = "server_connections"
        private const val NOTIFICATION = 23001
        private val requested = AtomicBoolean(false)

        fun start(context: Context): Boolean {
            if (!requested.compareAndSet(false, true)) return true
            return try {
                ContextCompat.startForegroundService(context, Intent(context, ConnectionService::class.java))
                true
            } catch (_: IllegalStateException) { requested.set(false); false }
              catch (_: SecurityException) { requested.set(false); false }
        }

        fun stop(context: Context) {
            if (requested.get()) context.stopService(Intent(context, ConnectionService::class.java))
        }
    }
}
