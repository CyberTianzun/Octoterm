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
    Error { message: String },
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
