//! agent 会话表:内存态,不落盘。
//!
//! server 重启后,下一个 hook 事件就会把会话重建出来;丢掉的只是历史,而历史不是
//! 这个功能的目标。
//!
//! **任何事件都能惰性创建会话**,不假设 `SessionStart` 一定先到 —— Task 3 的端到端
//! 里就观察到过 `-p` 模式下第一个到达的是 `UserPromptSubmit`。

use std::collections::{HashMap, VecDeque};
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
/// 运维方可以用同名环境变量固定它(和 `--token` 固定客户端 token 是同一个道理:
/// 不固定的话每次重启都换,只是 hook 这边不受影响 —— 值是 spawn 时现给的)。
/// 主要用途是端到端测试:外部起的 Claude 需要拿到和 server 一致的值。
pub fn hook_token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        std::env::var("OCTOTERM_HOOK_TOKEN")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string())
    })
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

/// 用户对一个挂起请求的答复。
///
/// `NoDecision` 不是「拒绝」:它表示我们不替用户做决定,让 agent 回落到它自己终端里
/// 的审批流程。超时、没人连着、客户端断开,都走这条 —— **宁可不作决定,也不代替
/// 用户 allow 或 deny**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow { message: Option<String> },
    Deny { message: Option<String> },
    NoDecision,
}

/// 一个正在等人回答的请求。
#[derive(Debug, Clone, Serialize)]
pub struct PendingRequest {
    pub id: String,
    pub agent_id: String,
    pub agent_session_id: String,
    pub session: Option<u64>,
    pub tool_name: Option<String>,
    /// 工具入参。可能很长(命令原文),客户端自己决定怎么截断展示。
    pub tool_input: serde_json::Value,
    pub created_at: u64,
}

struct PendingEntry {
    meta: PendingRequest,
    tx: Option<tokio::sync::oneshot::Sender<Decision>>,
    /// 已经有人答过了。留着这个标记(而不是直接删)是为了让第二次回答拿到 409
    /// 而不是 404 —— 「重复提交」和「这个请求根本不存在」对客户端是两件事。
    answered: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AnswerResult {
    Ok,
    NotFound,
    AlreadyAnswered,
}

/// 记住最近答过的 id 有多少个。
///
/// 为什么需要它:用户答完之后,挂在 socket 上的那个 handler 立刻醒来、写响应、
/// Drop guard 摘掉条目 —— 这中间只有微秒级的窗口。没有这份记录的话,客户端因为
/// 网络抖动重试一次,拿到的会是 404「这个请求根本不存在」,而事实是「你已经答过了」。
/// 这两件事对客户端不是一回事,不能混成同一个码。
const ANSWERED_MEMORY: usize = 64;

#[derive(Default)]
pub struct AgentSessionStore {
    sessions: Mutex<HashMap<(String, String), AgentSession>>,
    pending: Mutex<HashMap<String, PendingEntry>>,
    answered: Mutex<VecDeque<String>>,
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

    pub fn snapshot(&self, agent_id: &str, agent_session_id: &str) -> Option<AgentSession> {
        self.sessions
            .lock()
            .unwrap()
            .get(&(agent_id.to_string(), agent_session_id.to_string()))
            .cloned()
    }

    pub fn list(&self) -> Vec<AgentSession> {
        let mut v: Vec<_> = self.sessions.lock().unwrap().values().cloned().collect();
        v.sort_by(|a, b| {
            a.agent_id.cmp(&b.agent_id).then(a.agent_session_id.cmp(&b.agent_session_id))
        });
        v
    }

    /// 登记一个挂起请求,返回等待答复的接收端。
    ///
    /// 同时把对应会话置为 `Waiting` 并挂上 `pending` —— 客户端的「有事找你」就是
    /// 靠这两个字段。
    pub fn insert_pending(
        &self,
        meta: PendingRequest,
    ) -> tokio::sync::oneshot::Receiver<Decision> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let key = (meta.agent_id.clone(), meta.agent_session_id.clone());
        if let Some(s) = self.sessions.lock().unwrap().get_mut(&key) {
            s.state = AgentState::Waiting;
            s.pending = Some(meta.id.clone());
            s.updated_at = now();
        }
        self.pending
            .lock()
            .unwrap()
            .insert(meta.id.clone(), PendingEntry { meta, tx: Some(tx), answered: false });
        rx
    }

    /// 摘掉一个挂起请求。**由 handler 的 Drop guard 调用** —— agent 侧断开连接时
    /// axum 会把 handler future 丢掉,只有 Drop 能保证条目不会一直挂到超时。
    pub fn remove_pending(&self, id: &str) -> Option<PendingRequest> {
        let entry = self.pending.lock().unwrap().remove(id)?;
        let key = (entry.meta.agent_id.clone(), entry.meta.agent_session_id.clone());
        if let Some(s) = self.sessions.lock().unwrap().get_mut(&key)
            && s.pending.as_deref() == Some(id)
        {
            s.pending = None;
            // 不再等人了,但也说不上在干什么 —— 交给下一个 hook 事件去纠正
            s.state = AgentState::Working;
            s.updated_at = now();
        }
        Some(entry.meta)
    }

    pub fn answer(&self, id: &str, decision: Decision) -> AnswerResult {
        let mut guard = self.pending.lock().unwrap();
        let Some(entry) = guard.get_mut(id) else {
            // 条目不在了:可能是刚答完被摘掉,也可能是压根没有过
            return if self.answered.lock().unwrap().iter().any(|x| x == id) {
                AnswerResult::AlreadyAnswered
            } else {
                AnswerResult::NotFound
            };
        };
        if entry.answered {
            return AnswerResult::AlreadyAnswered;
        }
        entry.answered = true;
        let tx = entry.tx.take();
        drop(guard);
        self.remember_answered(id);
        match tx {
            // 接收端已经没了(agent 断开),答复无处可去 —— 但仍算「答过了」
            Some(tx) => {
                let _ = tx.send(decision);
                AnswerResult::Ok
            }
            None => AnswerResult::AlreadyAnswered,
        }
    }

    fn remember_answered(&self, id: &str) {
        let mut a = self.answered.lock().unwrap();
        a.push_back(id.to_string());
        while a.len() > ANSWERED_MEMORY {
            a.pop_front();
        }
    }

    pub fn list_pending(&self) -> Vec<PendingRequest> {
        let mut v: Vec<_> =
            self.pending.lock().unwrap().values().map(|e| e.meta.clone()).collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
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
