use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: u64,
    pub name: String,
    pub cols: u16,
    pub rows: u16,
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttachMode {
    Replay,
    Resync,
}

/// 一个 agent 会话此刻在干什么。
///
/// 刻意只有六档 —— 它要驱动的是「列表上一个状态点」和「有没有人在等你」,不是
/// 一套动画。参考实现 clawd-on-desk 有十一档,那是桌宠的需要,对客户端中立的
/// 协议是负担(R13)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentState {
    Idle,
    Thinking,
    Working,
    /// **在等人**。这是整条链路里唯一必须精确的状态:客户端的「有事找你」全靠它。
    Waiting,
    Done,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionEventKind {
    Created,
    Renamed,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ClientMsg {
    Hello { token: String, proto: u32 },
    ListSessions,
    /// `cwd` 是启动目录。之所以要它:客户端菜单里的启动项来自系统上已有终端的
    /// profile(见服务端 `launcher` 模块),而那些 profile 的"工作目录"和"命令"
    /// 是一体的 —— 只传 command 会把 Windows Terminal 的 `startingDirectory`、
    /// iTerm2 的 `Custom Directory` 悄悄丢掉,变成"读了配置但行为对不上"。
    /// 缺省 / 目录不存在时回落到服务端的默认启动目录。
    NewSession {
        name: Option<String>,
        command: Option<Vec<String>>,
        #[serde(default)]
        cwd: Option<String>,
    },
    KillSession { id: u64 },
    RenameSession { id: u64, name: String },
    Preview { id: u64 },
    Attach { id: u64, channel: u32, last_seq: Option<u64>, cols: u16, rows: u16 },
    Detach { channel: u32 },
    Resize { channel: u32, cols: u16, rows: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ServerMsg {
    HelloOk { proto: u32 },
    /// `channel` 有值时表示这个错误是针对某个具体 channel 的操作(attach/
    /// detach/resize/input)失败;省略(None)表示连接级/会话级错误,序列化时
    /// 直接不出现这个字段(旧客户端按缺省 None 解析,兼容)。
    Error {
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<u32>,
    },
    Sessions { sessions: Vec<SessionInfo> },
    SessionEvent { event: SessionEventKind, session: SessionInfo },
    /// data = base64 编码的 ANSI 重绘序列
    PreviewData { id: u64, data: String },
    /// seq 记账不变式(客户端必须遵守):
    ///
    /// 客户端只能从控制消息锚定 `last_seq` —— 要么是 `mode: Replay` 时自己发出的
    /// `last_seq`(重放数据不改变这个锚点,因为重放的字节本就是从该点续接的),
    /// 要么是 `ResyncEnd.seq`;此后每收到一条数据帧,把它的字节长度累加到
    /// `last_seq` 上。`Attached.seq` 只是"重放会在哪个 seq 结束"的参考信息,
    /// 绝不能被客户端当作锚点使用 —— 用它会与按字节推进的账本重复计数。
    Attached { channel: u32, seq: u64, mode: AttachMode },
    /// 会话的权威尺寸(G1)。pty 只有一个尺寸,而 `attach`/`resize` 只是本端的
    /// 尺寸诉求(G2);服务端按 window-size 策略归并所有 attach 之后用这条消息
    /// 通知每个 attach。客户端必须按这里的尺寸渲染,而不是自己请求的那个(G7)。
    Resized { channel: u32, cols: u16, rows: u16 },
    ResyncBegin { channel: u32 },
    /// resync 的权威锚点:重绘(repaint)字节是合成的,不计入 seq 账本;
    /// 客户端收到本消息后应把 `last_seq` 直接置为 `seq`,此后按数据帧字节长度累加。
    ResyncEnd { channel: u32, seq: u64 },
    SessionExited { channel: u32, id: u64 },
    /// 托管会话里的 coding agent 状态变了。
    ///
    /// 走 server→client 的新消息类型是兼容的(X2:客户端忽略未知 type),因此**不需要
    /// bump proto**。反方向(客户端回答一个 pending)刻意不走控制消息,而是
    /// `POST /api/agents/answer` —— 新增 client→server 类型按 X3 是破坏性变更,
    /// 为一个低频请求付「所有已打开页面全断」的代价不值。
    ///
    /// 只描述状态,不含任何窗口/标签/面板语义(R13):怎么渲染是客户端的事。
    AgentEvent {
        agent_id: String,
        /// agent 自己的会话标识(它在 hook payload 里给的 `session_id`)
        agent_session_id: String,
        /// 关联到的 octoterm 托管会话。目前恒有值 —— 拿不到关联的 hook 在鉴权那一层
        /// 就被挡掉了;留成 Option 是为了将来支持非托管会话时不必 bump proto(X4)。
        #[serde(default)]
        session: Option<u64>,
        state: AgentState,
        /// 有值表示正在等人回答,值是 `POST /api/agents/answer` 的自然键(C5/R5)。
        #[serde(default)]
        pending: Option<String>,
        /// 一行给人看的说明(在等什么/在跑什么工具)。展示用,客户端不解析。
        #[serde(default)]
        detail: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_msg_wire_format() {
        let msg = ClientMsg::Attach { id: 3, channel: 1, last_seq: Some(42), cols: 80, rows: 24 };
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "attach");
        assert_eq!(json["last_seq"], 42);
        let back: ClientMsg = serde_json::from_value(json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn fixtures_roundtrip() {
        let raw = include_str!("../fixtures/client-msgs.json");
        let msgs: Vec<ClientMsg> = serde_json::from_str(raw).unwrap();
        assert!(msgs.len() >= 5);
        let raw = include_str!("../fixtures/server-msgs.json");
        let msgs: Vec<ServerMsg> = serde_json::from_str(raw).unwrap();
        assert!(msgs.len() >= 5);
    }
}
