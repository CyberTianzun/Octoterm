//! agent 会话表:内存态,不落盘。
//!
//! server 重启后,下一个 hook 事件就会把会话重建出来;丢掉的只是历史,而历史不是
//! 这个功能的目标。
//!
//! **任何事件都能惰性创建会话**,不假设 `SessionStart` 一定先到 —— Task 3 的端到端
//! 里就观察到过 `-p` 模式下第一个到达的是 `UserPromptSubmit`。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use octoterm_protocol::{AgentState, ServerMsg};
use serde::Serialize;

/// 进程级 hook 密钥。
///
/// 为什么可以是进程级、不落盘:写进 `settings.json` 的是字面量
/// `$OCTOTERM_HOOK_TOKEN`,插值发生在 hook 触发的那一刻,取自 Claude 进程的环境,
/// 而那份环境是 octoterm 在 spawn 这个会话时给的。会话与 server 进程同生共死,
/// 不存在「老会话拿着旧密钥」的窗口。
///
/// 顺带一个恰好正确的副作用:**环境变量就是能力本身**。在 octoterm 之外启动的
/// Claude 拿不到这个变量,hook 照样触发,但没有 `Authorization` 头 —— 401 拒收。
/// 「只管托管会话」这条边界由机制保证,不需要额外的判别逻辑。
pub fn hook_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| uuid::Uuid::new_v4().simple().to_string())
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// adapter 把 agent 方言的 hook payload 归一化成这个。
#[derive(Debug, Clone, Default)]
pub struct Update {
    /// `None` 表示这个事件不改变状态(例如纯信息类事件)
    pub state: Option<AgentState>,
    pub detail: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSession {
    pub agent_id: String,
    pub agent_session_id: String,
    pub session: Option<u64>,
    pub state: AgentState,
    pub detail: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    /// 有值 = 正在等人回答。Task 6 接上阻塞式决策后由它填。
    pub pending: Option<String>,
    pub updated_at: u64,
    /// 用户「知道了」的时刻。清理的空闲基准取 `max(updated_at, acked_at)` ——
    /// 否则用户刚看过一眼的会话会因为「很久没有新事件」被扫掉。
    pub acked_at: u64,
}

impl AgentSession {
    pub fn to_msg(&self) -> ServerMsg {
        ServerMsg::AgentEvent {
            agent_id: self.agent_id.clone(),
            agent_session_id: self.agent_session_id.clone(),
            session: self.session,
            state: self.state,
            pending: self.pending.clone(),
            detail: self.detail.clone(),
        }
    }
}

#[derive(Default)]
pub struct AgentSessionStore {
    sessions: Mutex<HashMap<(String, String), AgentSession>>,
}

impl AgentSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// 应用一次更新,返回更新后的快照。会话不存在就地创建。
    pub fn apply(
        &self,
        agent_id: &str,
        agent_session_id: &str,
        session: Option<u64>,
        up: Update,
    ) -> AgentSession {
        let key = (agent_id.to_string(), agent_session_id.to_string());
        let mut guard = self.sessions.lock().unwrap();
        let entry = guard.entry(key).or_insert_with(|| AgentSession {
            agent_id: agent_id.to_string(),
            agent_session_id: agent_session_id.to_string(),
            session,
            state: AgentState::Idle,
            detail: None,
            cwd: None,
            title: None,
            pending: None,
            updated_at: now(),
            acked_at: 0,
        });
        if let Some(s) = up.state {
            entry.state = s;
        }
        // 只覆盖有值的字段:一个只带 tool_name 的事件不该把 cwd 抹掉
        if up.detail.is_some() {
            entry.detail = up.detail;
        }
        if up.cwd.is_some() {
            entry.cwd = up.cwd;
        }
        if up.title.is_some() {
            entry.title = up.title;
        }
        if session.is_some() {
            entry.session = session;
        }
        entry.updated_at = now();
        entry.clone()
    }

    pub fn list(&self) -> Vec<AgentSession> {
        let mut v: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| {
            a.agent_id.cmp(&b.agent_id).then(a.agent_session_id.cmp(&b.agent_session_id))
        });
        v
    }

    /// 托管会话没了 ⇒ 里面的 agent 必死。这是比任何超时都硬的证据。
    pub fn drop_by_session(&self, session: u64) -> Vec<AgentSession> {
        let mut guard = self.sessions.lock().unwrap();
        let dead: Vec<_> = guard
            .values()
            .filter(|s| s.session == Some(session))
            .map(|s| {
                let mut s = s.clone();
                s.state = AgentState::Done;
                s.pending = None;
                s
            })
            .collect();
        guard.retain(|_, s| s.session != Some(session));
        dead
    }
}
