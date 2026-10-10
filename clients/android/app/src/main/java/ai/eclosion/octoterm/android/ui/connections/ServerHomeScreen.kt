package ai.eclosion.octoterm.android.ui.connections

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.connection.ServerConnection
import ai.eclosion.octoterm.android.launcher.Launcher
import ai.eclosion.octoterm.android.wire.SessionInfo

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ServerHomeScreen(
    connection: ServerConnection,
    sessions: List<SessionInfo>,
    openSessionIds: List<Long>,
    reconnecting: Boolean,
    pendingRename: SessionInfo?,
    renameDraft: String,
    launcherMenu: LauncherMenuState,
    onRenameDraft: (String) -> Unit,
    onConfirmRename: () -> Unit,
    onDismissRename: () -> Unit,
    onBack: () -> Unit,
    onNewSession: () -> Unit,
    onDismissLaunchers: () -> Unit,
    onPickLauncher: (Launcher) -> Unit,
    onOpenSession: (Long) -> Unit,
    onRename: (SessionInfo) -> Unit,
    onKill: (Long) -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(connection.title()) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(
                            Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = stringResource(R.string.action_back),
                        )
                    }
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(onClick = onNewSession) {
                Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.session_new))
            }
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding),
        ) {
            if (reconnecting) {
                LinearProgressIndicator(Modifier.fillMaxWidth())
            }
            if (sessions.isEmpty()) {
                EmptySessions(
                    modifier = Modifier.fillMaxSize(),
                    onNewSession = onNewSession,
                )
            } else {
                LazyColumn(
                    modifier = Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(bottom = 88.dp),
                ) {
                    items(sessions, key = { it.id }) { session ->
                        SessionRow(
                            session = session,
                            open = session.id in openSessionIds,
                            onOpen = { onOpenSession(session.id) },
                            onRename = { onRename(session) },
                            onKill = { onKill(session.id) },
                        )
                        HorizontalDivider()
                    }
                }
            }
        }
    }

    if (pendingRename != null) {
        AlertDialog(
            onDismissRequest = onDismissRename,
            title = { Text(stringResource(R.string.session_rename_title)) },
            text = {
                OutlinedTextField(
                    value = renameDraft,
                    onValueChange = onRenameDraft,
                    singleLine = true,
                    label = { Text(stringResource(R.string.session_rename_label)) },
                )
            },
            confirmButton = {
                TextButton(onClick = onConfirmRename) {
                    Text(stringResource(R.string.action_save))
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissRename) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }

    if (launcherMenu.open) {
        LauncherSheet(
            menu = launcherMenu,
            onDismiss = onDismissLaunchers,
            onPick = onPickLauncher,
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LauncherSheet(
    menu: LauncherMenuState,
    onDismiss: () -> Unit,
    onPick: (Launcher) -> Unit,
) {
    var query by remember { mutableStateOf("") }
    val shown = menu.items.filter { launcher ->
        val q = query.trim()
        if (q.isEmpty()) true
        else {
            val name = launcher.name.ifBlank { "shell" }
            "$name ${launcher.detail} ${launcher.provider}".contains(q, ignoreCase = true)
        }
    }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Text(
            text = stringResource(R.string.session_new),
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
        )
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            singleLine = true,
            label = { Text(stringResource(R.string.launcher_filter)) },
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 24.dp),
        )
        if (menu.loading && menu.items.size <= 1) {
            Text(
                text = stringResource(R.string.launcher_loading),
                modifier = Modifier.padding(24.dp),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Column(
            Modifier
                .fillMaxWidth()
                .heightIn(max = 480.dp)
                .verticalScroll(rememberScrollState())
                .padding(bottom = 32.dp),
        ) {
            var group: String? = null
            shown.forEach { launcher ->
                if (launcher.provider != group) {
                    group = launcher.provider
                    Text(
                        text = providerLabel(launcher.provider),
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
                    )
                }
                val name = launcher.name.ifBlank { stringResource(R.string.launcher_default_name) }
                val detail = launcher.detail.ifBlank { stringResource(R.string.launcher_default_detail) }
                val line = if (launcher.cwd.isNullOrBlank()) detail else "$detail  ·  ${launcher.cwd}"
                ListItem(
                    headlineContent = { Text(name) },
                    supportingContent = { Text(line, maxLines = 2) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable { onPick(launcher) },
                )
            }
        }
    }
}

@Composable
private fun providerLabel(provider: String): String {
    return when (provider) {
        "builtin" -> stringResource(R.string.launcher_provider_builtin)
        "config" -> stringResource(R.string.launcher_provider_config)
        "iterm2" -> "iTerm2"
        "windows-terminal" -> "Windows Terminal"
        else -> provider
    }
}

@Composable
private fun EmptySessions(modifier: Modifier, onNewSession: () -> Unit) {
    Column(
        modifier = modifier.padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = stringResource(R.string.session_empty_title),
            style = MaterialTheme.typography.titleLarge,
            textAlign = TextAlign.Center,
        )
        Text(
            text = stringResource(R.string.session_empty_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 8.dp, bottom = 16.dp),
        )
        TextButton(onClick = onNewSession) {
            Text(stringResource(R.string.session_new))
        }
    }
}

@Composable
private fun SessionRow(
    session: SessionInfo,
    open: Boolean,
    onOpen: () -> Unit,
    onRename: () -> Unit,
    onKill: () -> Unit,
) {
    var menu by remember { mutableStateOf(false) }
    ListItem(
        headlineContent = { Text(session.name.ifBlank { "#${session.id}" }) },
        supportingContent = {
            val size = "${session.cols}×${session.rows}"
            Text(if (open) "$size · ${stringResource(R.string.session_open)}" else size)
        },
        trailingContent = {
            Box {
                IconButton(onClick = { menu = true }) {
                    Icon(Icons.Filled.MoreVert, contentDescription = stringResource(R.string.servers_more))
                }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.action_edit)) },
                        onClick = {
                            menu = false
                            onRename()
                        },
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.session_kill)) },
                        onClick = {
                            menu = false
                            onKill()
                        },
                    )
                }
            }
        },
        modifier = Modifier.clickable(onClick = onOpen),
    )
}
