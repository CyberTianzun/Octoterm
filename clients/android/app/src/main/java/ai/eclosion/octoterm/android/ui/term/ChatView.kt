package ai.eclosion.octoterm.android.ui.term

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import ai.eclosion.octoterm.android.R
import ai.eclosion.octoterm.android.agent.ChatBlock
import ai.eclosion.octoterm.android.agent.ChatFallback
import ai.eclosion.octoterm.android.agent.ChatMessage
import ai.eclosion.octoterm.android.agent.ChatRole
import ai.eclosion.octoterm.android.ui.connections.ChatNotice
import ai.eclosion.octoterm.android.ui.connections.ChatUiState

private val UserStripe = Color(0xFF4EA1FF)
private val SystemStripe = Color(0xFF9A8CFF)
private val ErrorColor = Color(0xFFFF6B6B)

/**
 * 聊天视图。和 web 端同一套呈现:每条消息左侧一道色条区分角色,思考过程默认折叠,
 * 工具调用和结果默认折叠,摘要保留命令、文件及失败状态,展开后可查看完整内容。
 *
 * 读不到记录时**说清原因并给出回终端的出路**(协议 A9)—— 空白的聊天框会被读成
 * 「没有对话」,比一句解释更糟。
 */
@Composable
fun ChatView(
    chat: ChatUiState,
    onOpenTerminal: () -> Unit,
    modifier: Modifier = Modifier,
    onLoadOlder: () -> Unit = {},
) {
    val notice = chat.notice
    when {
        notice != null -> Fallback(noticeText(notice), onOpenTerminal, modifier)
        chat.messages.isEmpty() && chat.loading -> Centered(modifier) {
            CircularProgressIndicator()
            Spacer(Modifier.height(12.dp))
            Text(stringResource(R.string.chat_loading), style = MaterialTheme.typography.bodyMedium)
        }
        chat.messages.isEmpty() && chat.olderCursor == null -> Centered(modifier) {
            Text(stringResource(R.string.chat_empty), style = MaterialTheme.typography.bodyMedium)
        }
        else -> key(chat.sessionId) { Messages(chat, onLoadOlder, modifier) }
    }
}

@Composable
private fun Messages(chat: ChatUiState, onLoadOlder: () -> Unit, modifier: Modifier) {
    val messages = chat.messages
    // The trailing item anchors the viewport below even a very tall final message.
    val listState = rememberLazyListState(initialFirstVisibleItemIndex = messages.size + 1)
    var historyPage by remember { mutableStateOf(chat.historyPage) }
    var historyAnchor by remember { mutableStateOf<Pair<String, Int>?>(null) }
    val olderLoading by rememberUpdatedState(chat.olderLoading)
    // 贴着底部时才自动跟随;用户往上翻看历史时不该被拽回来。只在**用户滚动**时更新,
    // 否则新消息一来「不在底部了」会先于跟随生效,把自己关掉。
    var follow by remember { mutableStateOf(true) }
    var scrollingToBottom by remember { mutableStateOf(false) }
    LaunchedEffect(listState) {
        snapshotFlow { Triple(listState.isScrollInProgress, listState.canScrollForward, scrollingToBottom) }
            .collect { (scrolling, canScrollForward, automatic) ->
                if (scrolling && !automatic) follow = !canScrollForward
            }
    }
    LaunchedEffect(listState) {
        snapshotFlow {
            if (olderLoading && listState.isScrollInProgress && !scrollingToBottom)
                listState.layoutInfo.visibleItemsInfo.firstOrNull { (it.key as? String)?.startsWith("message:") == true }
                    ?.let { it.key as String to it.offset }
            else null
        }.collect { anchor -> if (anchor != null) historyAnchor = anchor }
    }
    LaunchedEffect(messages, chat.historyPage) {
        if (historyPage != chat.historyPage) {
            historyPage = chat.historyPage
            val anchor = historyAnchor
            historyAnchor = null
            val index = anchor?.let { saved -> messages.indexOfFirst { "message:${it.id}" == saved.first } } ?: -1
            if (anchor != null && index >= 0) {
                scrollingToBottom = true
                try { listState.scrollToItem(index + 1, -anchor.second) }
                finally { scrollingToBottom = false }
            }
            return@LaunchedEffect
        }
        if (follow) {
            scrollingToBottom = true
            try { listState.scrollToItem(messages.size + 1) }
            finally { scrollingToBottom = false }
        }
    }
    val scrollbarColor = MaterialTheme.colorScheme.onSurfaceVariant
    LazyColumn(
        state = listState,
        modifier = modifier.fillMaxSize().chatScrollbar(listState, messages.size + 1, scrollbarColor),
        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item(key = "history-control") {
            if (chat.historySupported) Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
                if (chat.olderCursor != null) OutlinedButton(onClick = {
                    follow = false
                    historyAnchor = listState.layoutInfo.visibleItemsInfo
                        .firstOrNull { (it.key as? String)?.startsWith("message:") == true }
                        ?.let { it.key as String to it.offset }
                    onLoadOlder()
                }, enabled = !chat.olderLoading) {
                    Text(stringResource(when {
                        chat.olderLoading -> R.string.chat_loading_older
                        chat.olderFailed -> R.string.chat_older_failed
                        else -> R.string.chat_load_older
                    }))
                } else Text(stringResource(R.string.chat_beginning), style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(vertical = 8.dp))
            }
        }
        items(messages, key = { "message:${it.id}" }) { MessageRow(it) }
        item(key = "conversation-end") { Spacer(Modifier.height(1.dp)) }
    }
}

/** Keep a scroll indicator visible whenever the conversation exceeds the viewport. */
private fun Modifier.chatScrollbar(state: LazyListState, messageCount: Int, color: Color): Modifier = drawWithContent {
    drawContent()
    if ((!state.canScrollBackward && !state.canScrollForward) || messageCount == 0) return@drawWithContent
    val layout = state.layoutInfo
    val visible = layout.visibleItemsInfo.filter { it.index < messageCount }
    val first = visible.firstOrNull() ?: return@drawWithContent
    val viewportStart = layout.viewportStartOffset
    val viewportEnd = layout.viewportEndOffset
    // Use visible fractions so a single message taller than the viewport still has a thumb.
    val visibleFraction = visible.sumOf { item ->
        val shown = (minOf(item.offset + item.size, viewportEnd) - maxOf(item.offset, viewportStart)).coerceAtLeast(0)
        shown.toDouble() / item.size.coerceAtLeast(1)
    }.toFloat()
    val firstFraction = (viewportStart - first.offset).coerceIn(0, first.size).toFloat() / first.size.coerceAtLeast(1)
    val progress = when {
        !state.canScrollBackward -> 0f
        !state.canScrollForward -> 1f
        else -> ((first.index + firstFraction) / (messageCount - visibleFraction).coerceAtLeast(0.001f)).coerceIn(0f, 1f)
    }
    val inset = 4.dp.toPx()
    val trackHeight = (size.height - inset * 2).coerceAtLeast(0f)
    if (trackHeight == 0f) return@drawWithContent
    val thumbHeight = (trackHeight * visibleFraction / messageCount).coerceIn(minOf(24.dp.toPx(), trackHeight), trackHeight)
    val width = 4.dp.toPx()
    val x = size.width - width - 2.dp.toPx()
    val radius = CornerRadius(width / 2)
    drawRoundRect(color.copy(alpha = 0.12f), Offset(x, inset), Size(width, trackHeight), radius)
    drawRoundRect(color.copy(alpha = 0.7f), Offset(x, inset + (trackHeight - thumbHeight) * progress), Size(width, thumbHeight), radius)
}

@Composable
private fun MessageRow(message: ChatMessage) {
    val stripe = when (message.role) {
        ChatRole.User -> UserStripe
        ChatRole.Assistant -> MaterialTheme.colorScheme.outline.copy(alpha = 0.5f)
        ChatRole.System -> SystemStripe
    }
    Row(
        Modifier
            .fillMaxWidth()
            .height(IntrinsicSize.Min)
            .alpha(if (message.role == ChatRole.System) 0.8f else 1f),
    ) {
        Box(
            Modifier
                .width(3.dp)
                .fillMaxHeight()
                .background(stripe, RoundedCornerShape(2.dp)),
        )
        Column(
            Modifier
                .padding(start = 10.dp)
                .weight(1f),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            message.blocks.forEachIndexed { i, block -> BlockView(block, "${message.id}#$i") }
        }
    }
}

@Composable
private fun BlockView(block: ChatBlock, stateKey: String) {
    when (block) {
        is ChatBlock.Text -> SelectionContainer {
            Text(block.text, style = MaterialTheme.typography.bodyMedium)
        }
        is ChatBlock.Thinking -> Disclosure(
            stateKey = stateKey,
            label = stringResource(R.string.chat_thinking),
            modifier = Modifier.alpha(0.8f),
        ) {
            SelectionContainer {
                Text(block.text, style = MaterialTheme.typography.bodySmall)
            }
        }
        is ChatBlock.ToolUse -> Disclosure(
            stateKey = stateKey,
            label = block.name,
            preview = excerpt(block.input),
        ) {
            ToolBody(block.input)
        }
        is ChatBlock.ToolResult -> ToolResult(block, stateKey)
    }
}

@Composable
private fun ToolResult(block: ChatBlock.ToolResult, stateKey: String) {
    val bg = if (block.ok) {
        MaterialTheme.colorScheme.surfaceVariant
    } else {
        ErrorColor.copy(alpha = 0.15f)
    }
    Disclosure(
        stateKey = stateKey,
        label = stringResource(R.string.chat_tool_result),
        preview = excerpt(block.text),
        error = !block.ok,
        modifier = Modifier
            .fillMaxWidth()
            .background(bg, RoundedCornerShape(4.dp))
            .padding(horizontal = 6.dp),
    ) {
        ToolBody(block.text)
    }
}

private fun excerpt(text: String): String =
    text.lineSequence().firstOrNull { it.isNotBlank() }?.trim()?.take(160).orEmpty()

@Composable
private fun ToolBody(text: String) {
    SelectionContainer {
        Text(text, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
    }
}

@Composable
private fun Disclosure(
    stateKey: String,
    label: String,
    preview: String = "",
    error: Boolean = false,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    var expanded by rememberSaveable(stateKey) { mutableStateOf(false) }
    val action = stringResource(if (expanded) R.string.chat_show_less else R.string.chat_show_more)
    val state = stringResource(if (expanded) R.string.chat_expanded else R.string.chat_collapsed)
    Column(modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .semantics { stateDescription = state }
                .clickable(role = Role.Button, onClickLabel = action) { expanded = !expanded }
                .padding(vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                (if (expanded) "▾ " else "▸ ") + label,
                fontWeight = FontWeight.SemiBold,
                fontSize = 12.sp,
            )
            if (error) {
                Text(stringResource(R.string.chat_tool_result_error), color = ErrorColor, fontSize = 11.sp)
            }
            Text(
                preview,
                fontSize = 12.sp,
                fontFamily = FontFamily.Monospace,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).alpha(0.75f),
            )
        }
        if (expanded) {
            Box(Modifier.padding(start = 12.dp, bottom = 6.dp)) { content() }
        }
    }
}

@Composable
private fun Fallback(text: String, onOpenTerminal: () -> Unit, modifier: Modifier) {
    Column(
        modifier
            .fillMaxSize()
            .padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(text, style = MaterialTheme.typography.bodyMedium)
        OutlinedButton(onClick = onOpenTerminal) {
            Text(stringResource(R.string.chat_open_terminal_instead))
        }
    }
}

@Composable
private fun Centered(modifier: Modifier, content: @Composable () -> Unit) {
    Column(
        modifier
            .fillMaxSize()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) { content() }
}

/** `unsupported-agent` 带上是哪一家:「这个 AI 读不了」比「读不了」有用得多。 */
@Composable
private fun noticeText(notice: ChatNotice): String = when (notice) {
    ChatNotice.NoAgent -> stringResource(R.string.chat_fallback_no_agent)
    is ChatNotice.Server -> when (notice.reason) {
        ChatFallback.Disabled -> stringResource(R.string.chat_fallback_disabled)
        ChatFallback.NoTranscriptPath -> stringResource(R.string.chat_fallback_no_path)
        ChatFallback.UnsupportedAgent -> stringResource(R.string.chat_fallback_unsupported, notice.agentId)
        ChatFallback.Unreadable -> stringResource(R.string.chat_fallback_unreadable)
        ChatFallback.ParseFailed -> stringResource(R.string.chat_fallback_parse_failed)
    }
}
