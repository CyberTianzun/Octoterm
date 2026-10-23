package ai.eclosion.octoterm.android

import android.Manifest
import android.content.Intent
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LifecycleEventEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import ai.eclosion.octoterm.android.cloud.CloudAccountViewModel
import ai.eclosion.octoterm.android.i18n.LocaleStore
import ai.eclosion.octoterm.android.push.AgentNotifications
import ai.eclosion.octoterm.android.push.PushRegistrar
import ai.eclosion.octoterm.android.push.PushRegistration
import ai.eclosion.octoterm.android.term.AgentKeyboardLayouts
import ai.eclosion.octoterm.android.ui.appearance.AppearanceScreen
import ai.eclosion.octoterm.android.ui.cloud.CloudAccountScreen
import ai.eclosion.octoterm.android.ui.connections.AppScreen
import ai.eclosion.octoterm.android.ui.connections.ConnectionEditorScreen
import ai.eclosion.octoterm.android.ui.connections.ConnectionListScreen
import ai.eclosion.octoterm.android.ui.connections.ConnectionViewModel
import ai.eclosion.octoterm.android.ui.connections.ServerHomeScreen
import ai.eclosion.octoterm.android.ui.connections.UserMessage
import ai.eclosion.octoterm.android.ui.term.TerminalScreen
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive

@Composable
fun OctotermApp(viewModel: ConnectionViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val context = LocalContext.current
    var localePref by remember { mutableStateOf(LocaleStore.load(context)) }
    // The ViewModel keeps the native account lazy for users who have never signed in.
    val cloudViewModel: CloudAccountViewModel = viewModel(factory = CloudAccountViewModel.Factory)
    val cloud by cloudViewModel.state.collectAsStateWithLifecycle()
    val pushRegistration by PushRegistrar.state.collectAsStateWithLifecycle()
    val pushFailureCode by PushRegistrar.failureCode.collectAsStateWithLifecycle()
    val pushAvailable = remember { PushRegistrar.pushAvailable(context) }
    val permissionPreferences = remember { context.getSharedPreferences("notifications", Context.MODE_PRIVATE) }
    var notificationCheck by remember { mutableIntStateOf(0) }
    val permissionLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        notificationCheck++
    }
    // Also handles upgrading a previously signed-in APK that did not contain Firebase configuration.
    val hasBackgroundConnections = state.keepConnectionsInBackground && state.connectionStatuses.values.any { it.active }
    LaunchedEffect(cloud.status?.email, cloud.askNotificationPermission, pushRegistration, hasBackgroundConnections) {
        val explicitRequest = cloud.askNotificationPermission
        if (explicitRequest) cloudViewModel.consumePermissionRequest()
        if (hasBackgroundConnections || (cloud.status != null && pushAvailable && pushRegistration.registered)) {
            AgentNotifications.ensureChannel(context)
            if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context,
                    Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED &&
                (explicitRequest || !permissionPreferences.getBoolean("permission_requested", false))) {
                permissionPreferences.edit().putBoolean("permission_requested", true).apply()
                permissionLauncher.launch(Manifest.permission.POST_NOTIFICATIONS)
            }
        }
    }
    val lifecycleOwner = LocalLifecycleOwner.current
    LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
        if (cloud.status != null && pushRegistration != PushRegistration.Registering) {
            PushRegistrar.refreshOnStart(context)
        }
    }
    LaunchedEffect(cloudViewModel, lifecycleOwner) {
        lifecycleOwner.lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                cloudViewModel.refreshStatus()
                delay(10000)
            }
        }
    }
    LaunchedEffect(cloud.browserLaunch) {
        cloud.browserLaunch?.let { url ->
            cloudViewModel.consumeBrowserLaunch()
            runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
                .onFailure { cloudViewModel.browserUnavailable() }
        }
    }

    LaunchedEffect(viewModel) {
        viewModel.messages.collect { message ->
            snackbar.showSnackbar(context.messageText(message))
        }
    }

    // Edge-to-edge needs IME insets so the terminal and its keys stay above the keyboard.
    Box(Modifier.fillMaxSize().imePadding()) {
        when (val screen = state.screen) {
            AppScreen.List -> {
                ConnectionListScreen(
                    state = state,
                    onAdd = viewModel::openAddSheet,
                    onDismissAddSheet = viewModel::dismissAddSheet,
                    onAddFromClipboard = viewModel::addFromClipboard,
                    onAddManually = viewModel::addManually,
                    onConnect = viewModel::connect,
                    onReconnect = viewModel::reconnect,
                    onDisconnect = viewModel::disconnectServer,
                    onEdit = viewModel::edit,
                    onRequestDelete = viewModel::requestDelete,
                    onConfirmDelete = viewModel::confirmDelete,
                    onDismissDelete = viewModel::dismissDelete,
                    onSettings = viewModel::openAppearance,
                    onCloudAccount = viewModel::openCloudAccount,
                    cloudEmail = cloud.status?.email,
                )
            }
            AppScreen.Appearance -> {
                BackHandler(onBack = viewModel::closeAppearance)
                AppearanceScreen(
                    appearance = state.appearance,
                    followsSystem = state.appearanceFollowsSystem,
                    onFollowSystem = viewModel::followSystemTheme,
                    onBack = viewModel::closeAppearance,
                    onFontSize = viewModel::setFontSize,
                    onFont = viewModel::setFont,
                    onScrollback = viewModel::setScrollback,
                    onTheme = viewModel::selectTheme,
                    onCopy = viewModel::copyAppearance,
                    onImport = viewModel::importAppearanceFromClipboard,
                    localePref = localePref,
                    onLocalePref = { pref ->
                        LocaleStore.save(context, pref)
                        localePref = pref
                        LocaleStore.apply(pref)
                    },
                    extraKeyRows = state.extraKeyRows,
                    onExtraKeyRows = viewModel::setExtraKeyRows,
                    agentKeyLayouts = state.agentKeyLayouts,
                    onAgentKeyLayout = viewModel::setAgentKeyLayout,
                    keepConnectionsInBackground = state.keepConnectionsInBackground,
                    onKeepConnectionsInBackground = viewModel::setKeepConnectionsInBackground,
                )
            }
            AppScreen.CloudAccount -> {
                BackHandler(onBack = viewModel::closeCloudAccount)
                // 从系统设置/权限弹窗回来时重新读通知开关;账号状态也可能在别处变了(如 token 失效)。
                LifecycleEventEffect(Lifecycle.Event.ON_RESUME) {
                    notificationCheck++
                    cloudViewModel.refreshStatus()
                }
                val notificationsEnabled = remember(cloud.status, cloud.message, notificationCheck) {
                    AgentNotifications.enabled(context)
                }
                CloudAccountScreen(
                    state = cloud,
                    pushAvailable = pushAvailable,
                    notificationsEnabled = notificationsEnabled,
                    pushRegistration = pushRegistration,
                    pushFailureCode = pushFailureCode,
                    onRetryPush = cloudViewModel::retryPushRegistration,
                    onForm = cloudViewModel::updateForm,
                    onAction = cloudViewModel::run,
                    onOpenNotificationSettings = {
                        context.startActivity(
                            Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS).putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName),
                        )
                    },
                    onBack = viewModel::closeCloudAccount,
                )
            }
            is AppScreen.Editor -> {
                BackHandler(onBack = viewModel::closeEditor)
                ConnectionEditorScreen(
                    editing = screen.connectionId != null,
                    state = state.editor,
                    onNameChange = viewModel::onNameChange,
                    onUrlChange = viewModel::onUrlChange,
                    onTokenChange = viewModel::onTokenChange,
                    onPasteClipboard = viewModel::pasteClipboardIntoEditor,
                    onSave = viewModel::saveEditor,
                    onBack = viewModel::closeEditor,
                )
            }
            is AppScreen.Server -> {
                val connection = viewModel.connection(screen.connectionId)
                if (connection == null) {
                    viewModel.leaveServer()
                } else if (state.attachedId != null && viewModel.activeTerm() != null) {
                    val activeId = state.attachedId
                    val term = viewModel.activeTerm()
                    if (activeId == null || term == null) {
                        viewModel.leaveTerminal()
                    } else TerminalScreen(
                        title = viewModel.attachedSession()?.name ?: connection.title(),
                        activeId = activeId,
                        reconnecting = state.reconnecting,
                        serverOs = state.serverOs,
                        term = term,
                        frame = viewModel.painted,
                        generation = state.termGeneration,
                        appearance = state.appearance,
                        extraKeyRows = state.extraKeyRows,
                        agentId = AgentKeyboardLayouts.agentForTerminal(state.agents, activeId),
                        agentKeyLayouts = state.agentKeyLayouts,
                        selection = state.selection,
                        scrollOffset = state.scrollOffset,
                        mouseTracking = state.mouseTracking,
                        onBack = viewModel::leaveTerminal,
                        onInput = { bytes -> viewModel.sendInput(activeId, bytes) },
                        onProposeSize = viewModel::proposeSize,
                        onScroll = viewModel::scrollBy,
                        onSelection = viewModel::setSelection,
                        onMouse = viewModel::sendMouse,
                        onCopy = viewModel::copySelection,
                        onPaste = viewModel::pasteClipboard,
                        onJumpToBottom = viewModel::jumpToBottom,
                        chat = state.chat,
                        chatOpen = state.chatOpen,
                        chatAvailable = state.chatAvailable,
                        onToggleChat = viewModel::toggleChat,
                        onLoadOlderChat = viewModel::loadOlderChat,
                    )
                } else {
                    BackHandler(onBack = viewModel::leaveServer)
                    ServerHomeScreen(
                        connection = connection,
                        sessions = state.sessions,
                        agents = state.agents,
                        openSessionIds = state.openSessionIds,
                        reconnecting = state.reconnecting,
                        pendingRename = state.pendingRename,
                        renameDraft = state.renameDraft,
                        launcherMenu = state.launcherMenu,
                        onRenameDraft = viewModel::onRenameDraft,
                        onConfirmRename = viewModel::confirmRename,
                        onDismissRename = viewModel::dismissRename,
                        onBack = viewModel::leaveServer,
                        onNewSession = viewModel::openLauncherMenu,
                        onDismissLaunchers = viewModel::dismissLauncherMenu,
                        onPickLauncher = viewModel::newSession,
                        onOpenSession = viewModel::openSession,
                        onRename = viewModel::requestRename,
                        onKill = viewModel::killSession,
                        onLoadHistory = viewModel::loadHistory,
                        onPreviewHistory = viewModel::previewHistory,
                        onResumeHistory = viewModel::resumeHistory,
                    )
                }
            }
        }
        SnackbarHost(
            hostState = snackbar,
            modifier = Modifier
                .align(Alignment.BottomCenter)
                .navigationBarsPadding(),
        )
    }
}

private fun android.content.Context.messageText(message: UserMessage): String {
    return when (message) {
        UserMessage.Saved -> getString(R.string.msg_saved)
        UserMessage.Deleted -> getString(R.string.msg_deleted)
        is UserMessage.AuthFailed -> {
            val detail = message.serverDetail?.trim().orEmpty()
            if (detail.isEmpty()) {
                getString(R.string.msg_auth_failed)
            } else {
                getString(R.string.msg_auth_failed_detail, detail)
            }
        }
        UserMessage.Unreachable -> getString(R.string.msg_unreachable)
        UserMessage.Timeout -> getString(R.string.msg_timeout)
        UserMessage.Closed -> getString(R.string.msg_closed)
        UserMessage.Unexpected -> getString(R.string.msg_unexpected)
        UserMessage.BackgroundUnavailable -> getString(R.string.connections_background_unavailable)
        UserMessage.Copied -> getString(R.string.msg_copied)
        UserMessage.ImportFailed -> getString(R.string.msg_import_failed)
        UserMessage.TooManySessions -> getString(R.string.msg_too_many_sessions)
        UserMessage.ConnectServerFirst -> getString(R.string.msg_connect_server_first)
        UserMessage.SessionGone -> getString(R.string.msg_session_gone)
    }
}
