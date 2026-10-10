package ai.eclosion.octoterm.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import ai.eclosion.octoterm.android.i18n.LocaleStore
import ai.eclosion.octoterm.android.ui.appearance.AppearanceScreen
import ai.eclosion.octoterm.android.ui.connections.AppScreen
import ai.eclosion.octoterm.android.ui.connections.ConnectionEditorScreen
import ai.eclosion.octoterm.android.ui.connections.ConnectionListScreen
import ai.eclosion.octoterm.android.ui.connections.ConnectionViewModel
import ai.eclosion.octoterm.android.ui.connections.ServerHomeScreen
import ai.eclosion.octoterm.android.ui.connections.UserMessage
import ai.eclosion.octoterm.android.ui.term.TerminalScreen

@Composable
fun OctotermApp(viewModel: ConnectionViewModel) {
    val state by viewModel.state.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    val context = LocalContext.current
    var localePref by remember { mutableStateOf(LocaleStore.load(context)) }

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
                    onEdit = viewModel::edit,
                    onRequestDelete = viewModel::requestDelete,
                    onConfirmDelete = viewModel::confirmDelete,
                    onDismissDelete = viewModel::dismissDelete,
                    localePref = localePref,
                    onLocalePref = { pref ->
                        LocaleStore.save(context, pref)
                        localePref = pref
                        LocaleStore.apply(pref)
                    },
                    onAppearance = viewModel::openAppearance,
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
                    viewModel.disconnect()
                } else if (state.attachedId != null && viewModel.activeEmulator() != null) {
                    val activeId = state.attachedId
                    val emulator = viewModel.activeEmulator()
                    if (activeId == null || emulator == null) {
                        viewModel.leaveTerminal()
                    } else TerminalScreen(
                        title = viewModel.attachedSession()?.name ?: connection.title(),
                        tabs = viewModel.openTabs(),
                        activeId = activeId,
                        reconnecting = state.reconnecting,
                        serverOs = state.serverOs,
                        emulator = emulator,
                        frame = viewModel.painted,
                        generation = state.termGeneration,
                        appearance = state.appearance,
                        selection = state.selection,
                        scrollOffset = state.scrollOffset,
                        mouseTracking = state.mouseTracking,
                        onBack = viewModel::leaveTerminal,
                        onFocus = viewModel::openSession,
                        onCloseTab = viewModel::closeOpenSession,
                        onInput = { bytes -> viewModel.sendInput(activeId, bytes) },
                        onProposeSize = viewModel::proposeSize,
                        onScroll = viewModel::scrollBy,
                        onSelection = viewModel::setSelection,
                        onMouse = viewModel::sendMouse,
                        onCopy = viewModel::copySelection,
                        onPaste = viewModel::pasteClipboard,
                        onJumpToBottom = viewModel::jumpToBottom,
                    )
                } else {
                    BackHandler(onBack = viewModel::disconnect)
                    ServerHomeScreen(
                        connection = connection,
                        sessions = state.sessions,
                        openSessionIds = state.openSessionIds,
                        reconnecting = state.reconnecting,
                        pendingRename = state.pendingRename,
                        renameDraft = state.renameDraft,
                        launcherMenu = state.launcherMenu,
                        onRenameDraft = viewModel::onRenameDraft,
                        onConfirmRename = viewModel::confirmRename,
                        onDismissRename = viewModel::dismissRename,
                        onBack = viewModel::disconnect,
                        onNewSession = viewModel::openLauncherMenu,
                        onDismissLaunchers = viewModel::dismissLauncherMenu,
                        onPickLauncher = viewModel::newSession,
                        onOpenSession = viewModel::openSession,
                        onRename = viewModel::requestRename,
                        onKill = viewModel::killSession,
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
        UserMessage.Copied -> getString(R.string.msg_copied)
        UserMessage.ImportFailed -> getString(R.string.msg_import_failed)
        UserMessage.TooManySessions -> getString(R.string.msg_too_many_sessions)
    }
}
