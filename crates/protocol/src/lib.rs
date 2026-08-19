//! octoterm 线上协议的类型定义。
//!
//! 规范性文档:`docs/protocol.md`(帧格式、消息目录、seq 记账不变式、兼容性
//! 规则,以及新增/修改消息必须过的评审清单)。改动本文件前先读那份文档的
//! §11 兼容性与 §12 评审清单,并在同一个 PR 里同步更新文档与 fixtures。

pub mod frame;
pub use frame::{Frame, FrameError, CONTROL_CHANNEL};

pub mod messages;
pub use messages::{AgentState, AttachMode, ClientMsg, ServerMsg, SessionEventKind, SessionInfo};

pub const PROTO_VERSION: u32 = 1;
