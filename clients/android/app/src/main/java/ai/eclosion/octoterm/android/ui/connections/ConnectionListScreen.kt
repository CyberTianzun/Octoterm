package ai.eclosion.octoterm.android.ui.connections

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
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
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.connection.ServerConnection
import ai.eclosion.octoterm.android.i18n.LocalePref
import androidx.compose.foundation.background

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ConnectionListScreen(
    state: ConnectionUiState,
    onAdd: () -> Unit,
    onDismissAddSheet: () -> Unit,
    onAddFromClipboard: () -> Unit,
    onAddManually: () -> Unit,
    onConnect: (String) -> Unit,
    onEdit: (String) -> Unit,
    onRequestDelete: (ServerConnection) -> Unit,
    onConfirmDelete: () -> Unit,
    onDismissDelete: () -> Unit,
    localePref: LocalePref,
    onLocalePref: (LocalePref) -> Unit,
    onAppearance: () -> Unit,
) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(stringResource(R.string.servers_title)) },
                actions = {
                    TextButton(onClick = onAppearance) {
                        Text(stringResource(R.string.appearance_action))
                    }
                    LanguageMenu(localePref, onLocalePref)
                },
            )
        },
        floatingActionButton = {
            FloatingActionButton(onClick = onAdd) {
                Icon(Icons.Filled.Add, contentDescription = stringResource(R.string.servers_add))
            }
        },
    ) { padding ->
        if (state.connections.isEmpty()) {
            EmptyServers(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
                onAdd = onAdd,
            )
        } else {
            LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
                contentPadding = PaddingValues(bottom = 88.dp),
            ) {
                items(state.connections, key = { it.id }) { item ->
                    ServerRow(
                        connection = item,
                        connecting = state.connectingId == item.id,
                        enabled = state.connectingId == null,
                        onConnect = { onConnect(item.id) },
                        onEdit = { onEdit(item.id) },
                        onDelete = { onRequestDelete(item) },
                    )
                    HorizontalDivider()
                }
            }
        }
    }

    if (state.showAddSheet) {
        ModalBottomSheet(
            onDismissRequest = onDismissAddSheet,
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Text(
                text = stringResource(R.string.servers_add),
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp),
            )
            state.editor.clipboardUrl?.let { clip ->
                ListItem(
                    headlineContent = { Text(stringResource(R.string.servers_add_clipboard)) },
                    supportingContent = { Text(clip, maxLines = 2) },
                    modifier = Modifier
                        .fillMaxWidth()
                        .clickable(onClick = onAddFromClipboard),
                )
            }
            ListItem(
                headlineContent = { Text(stringResource(R.string.servers_add_manual)) },
                supportingContent = { Text(stringResource(R.string.servers_add_manual_hint)) },
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(onClick = onAddManually),
            )
            Box(Modifier.padding(bottom = 24.dp))
        }
    }

    state.pendingDelete?.let { target ->
        AlertDialog(
            onDismissRequest = onDismissDelete,
            title = { Text(stringResource(R.string.servers_delete_title)) },
            text = { Text(stringResource(R.string.servers_delete_body, target.title())) },
            confirmButton = {
                TextButton(onClick = onConfirmDelete) {
                    Text(stringResource(R.string.action_delete))
                }
            },
            dismissButton = {
                TextButton(onClick = onDismissDelete) {
                    Text(stringResource(R.string.action_cancel))
                }
            },
        )
    }
}

@Composable
private fun EmptyServers(modifier: Modifier = Modifier, onAdd: () -> Unit) {
    Column(
        modifier = modifier.padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            text = stringResource(R.string.servers_empty_title),
            style = MaterialTheme.typography.titleLarge,
            textAlign = TextAlign.Center,
        )
        Text(
            text = stringResource(R.string.servers_empty_body),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 8.dp, bottom = 24.dp),
        )
        TextButton(onClick = onAdd) {
            Text(stringResource(R.string.servers_add))
        }
    }
}

@Composable
private fun ServerRow(
    connection: ServerConnection,
    connecting: Boolean,
    enabled: Boolean,
    onConnect: () -> Unit,
    onEdit: () -> Unit,
    onDelete: () -> Unit,
) {
    var menuOpen by remember { mutableStateOf(false) }
    val title = connection.title()
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(connection.subtitle()) },
        leadingContent = {
            Box(
                modifier = Modifier
                    .size(40.dp)
                    .clip(CircleShape)
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = title.firstOrNull()?.uppercase() ?: "S",
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    fontWeight = FontWeight.Medium,
                )
            }
        },
        trailingContent = {
            if (connecting) {
                CircularProgressIndicator(modifier = Modifier.size(24.dp), strokeWidth = 2.dp)
            } else {
                Box {
                    IconButton(onClick = { menuOpen = true }, enabled = enabled) {
                        Icon(Icons.Filled.MoreVert, contentDescription = stringResource(R.string.servers_more))
                    }
                    DropdownMenu(expanded = menuOpen, onDismissRequest = { menuOpen = false }) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.action_edit)) },
                            onClick = {
                                menuOpen = false
                                onEdit()
                            },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.action_delete)) },
                            onClick = {
                                menuOpen = false
                                onDelete()
                            },
                        )
                    }
                }
            }
        },
        modifier = Modifier.clickable(enabled = enabled && !connecting, onClick = onConnect),
    )
}

@Composable
private fun LanguageMenu(
    localePref: LocalePref,
    onLocalePref: (LocalePref) -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { open = true }) {
            Text(stringResource(R.string.locale_action))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            LocalePref.entries.forEach { pref ->
                DropdownMenuItem(
                    text = { Text(stringResource(pref.labelRes)) },
                    onClick = {
                        open = false
                        onLocalePref(pref)
                    },
                    trailingIcon = if (pref == localePref) {
                        { Icon(Icons.Filled.Check, contentDescription = null) }
                    } else {
                        null
                    },
                )
            }
        }
    }
}

private val LocalePref.labelRes: Int
    get() = when (this) {
        LocalePref.Auto -> R.string.locale_auto
        LocalePref.ZhCN -> R.string.locale_zh
        LocalePref.En -> R.string.locale_en
    }
