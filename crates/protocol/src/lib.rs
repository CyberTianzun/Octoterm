pub mod frame;
pub use frame::{Frame, FrameError, CONTROL_CHANNEL};

pub mod messages;
pub use messages::{AttachMode, ClientMsg, ServerMsg, SessionEventKind, SessionInfo};

pub const PROTO_VERSION: u32 = 1;
