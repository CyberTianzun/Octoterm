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
    NewSession { name: Option<String>, command: Option<Vec<String>> },
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
