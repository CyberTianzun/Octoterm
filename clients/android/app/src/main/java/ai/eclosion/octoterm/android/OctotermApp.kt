package ai.eclosion.octoterm.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
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

    Box(Modifier.fillMaxSize()) {
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
                } else if (state.attachedId != null) {
                    TerminalScreen(
                        title = viewModel.attachedSession()?.name ?: connection.title(),
                        reconnecting = state.reconnecting,
                        emulator = viewModel.emulator,
                        frame = viewModel.painted,
                        generation = state.termGeneration,
                        onBack = viewModel::closeTerminal,
                        onInput = viewModel::sendInput,
                        onProposeSize = viewModel::proposeSize,
                    )
                } else {
                    BackHandler(onBack = viewModel::disconnect)
                    ServerHomeScreen(
                        connection = connection,
                        sessions = state.sessions,
                        reconnecting = state.reconnecting,
                        pendingRename = state.pendingRename,
                        renameDraft = state.renameDraft,
                        onRenameDraft = viewModel::onRenameDraft,
                        onConfirmRename = viewModel::confirmRename,
                        onDismissRename = viewModel::dismissRename,
                        onBack = viewModel::disconnect,
                        onNewSession = viewModel::newSession,
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
    }
}
