//! agent 会话表:内存态,不落盘。
//!
//! server 重启后,下一个 hook 事件就会把会话重建出来;丢掉的只是历史,而历史不是
//! 这个功能的目标。
//!
//! **任何事件都能惰性创建会话**,不假设 `SessionStart` 一定先到 —— Task 3 的端到端
//! 里就观察到过 `-p` 模式下第一个到达的是 `UserPromptSubmit`。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
    Allow {
        message: Option<String>,
        /// 改写后的工具入参,原样回传给 agent。
        ///
        /// 这是**回答选择题**的机制:`AskUserQuestion` 这类工具走的也是
        /// `PermissionRequest` 通道,答案不是 allow/deny,而是「放行,并且把用户选的
        /// 答案填进入参里」。参见 `claude_code::render`。
        updated_input: Option<serde_json::Value>,
    },
    Deny {
        message: Option<String>,
    },
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
    /// 什么时候作废(unix 秒)。**必须由服务端算好给出去** —— 超时长度是服务端配置,
    /// 客户端不知道也不该猜;而「我还有多久」正是决定要不要现在处理的关键信息。
    pub expires_at: u64,
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
    sweeper_started: AtomicBool,
}

/// 对一个 agent 会话的清理判决。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sweep {
    /// 从表里删掉
    Drop,
    /// 状态显示成 idle。用于「卡在 working 上很久了」—— 它多半已经不在跑了,
    /// 但还不到该删的程度。
    MarkIdle,
}

/// 清理判决。**纯函数**:`now` 与「托管会话是否还活着」都由调用方给,
/// 这里不读时钟、不查进程,于是可以逐条断言而不引入时间相关的 flaky。
///
/// 规则的顺序是有讲究的:
///
/// 1. **托管会话没了 ⇒ 立即删**。pty 子进程退出意味着里面的 agent 必死,这是比任何
///    超时都硬的证据,所以排在一切时间判断之前。
/// 2. **正在等人的一律不扫**。人可能在睡觉。挂起请求自己有超时(`pending_timeout_secs`),
///    到点了会把 `pending` 摘掉,那时这条会话才重新进入清理的视野。
/// 3. 空闲基准取 `max(updated_at, acked_at)` —— 用户刚看过一眼的会话不该因为
///    「很久没有新事件」被扫掉。
///
/// 注意 `MarkIdle` **不刷新时间戳**。参考实现在这里踩过坑:刷新之后「超时转 idle」
/// 会不断把删除的时刻往后推,结果那条会话永远删不掉。这里让两个时钟都从同一个
/// 基准走,`MarkIdle` 只改显示,不续命。
pub fn decide(
    now: u64,
    s: &AgentSession,
    host_alive: bool,
    session_stale_secs: u64,
    working_stale_secs: u64,
) -> Option<Sweep> {
    if !host_alive {
        return Some(Sweep::Drop);
    }
    if s.pending.is_some() {
        return None;
    }
    let age = now.saturating_sub(s.updated_at.max(s.acked_at));
    if age > session_stale_secs {
        return Some(Sweep::Drop);
    }
    if age > working_stale_secs
        && matches!(s.state, AgentState::Working | AgentState::Thinking)
    {
        return Some(Sweep::MarkIdle);
    }
    None
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

    /// 跑一轮清理,返回需要广播出去的变化。
    ///
    /// `host_alive` 由调用方提供(问 `SessionManager` 那个托管会话还在不在)——
    /// 保持这个模块不依赖 session 层。
    pub fn sweep(
        &self,
        now: u64,
        host_alive: &dyn Fn(u64) -> bool,
        session_stale_secs: u64,
        working_stale_secs: u64,
    ) -> Vec<AgentSession> {
        let mut changed = Vec::new();
        let mut guard = self.sessions.lock().unwrap();
        guard.retain(|_, s| {
            let alive = s.session.map(host_alive).unwrap_or(true);
            match decide(now, s, alive, session_stale_secs, working_stale_secs) {
                Some(Sweep::Drop) => {
                    let mut gone = s.clone();
                    gone.state = AgentState::Done;
                    gone.pending = None;
                    changed.push(gone);
                    false
                }
                Some(Sweep::MarkIdle) => {
                    s.state = AgentState::Idle;
                    // 刻意不动 updated_at:见 `decide` 的文档
                    changed.push(s.clone());
                    true
                }
                None => true,
            }
        });
        changed
    }

    /// 起一个后台清理任务。**只会真正启动一次** —— desktop 重建 HTTP 层时
    /// `serve()` 会再调一次,不能因此多出一个扫描器。
    pub fn start_sweeper(
        self: &Arc<Self>,
        manager: Arc<crate::session::manager::SessionManager>,
        session_stale_secs: u64,
        working_stale_secs: u64,
    ) {
        if self.sweeper_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let store = Arc::downgrade(self);
        tokio::spawn(async move {
            // interval 的第一跳是**立即**触发的,而这个任务的首次轮询会被推迟到
            // 运行时下一次调度 —— 于是「立即」有可能落在启动之后的任意一点,把刚
            // 建好的会话当场扫掉。用 interval_at 把第一跳推到一个周期之后,启动
            // 瞬间不做任何判断。
            let period = std::time::Duration::from_secs(10);
            let mut tick =
                tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            loop {
                tick.tick().await;
                // store 没人要了就退出,不留悬空任务
                let Some(store) = store.upgrade() else { break };
                let alive = |id: u64| manager.get(id).is_some();
                for s in store.sweep(now(), &alive, session_stale_secs, working_stale_secs) {
                    manager.publish(s.to_msg());
                }
            }
        });
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


#[cfg(test)]
mod tests {
    use super::*;

    fn session(state: AgentState, updated_at: u64) -> AgentSession {
        AgentSession {
            agent_id: "claude-code".into(),
            agent_session_id: "s1".into(),
            session: Some(1),
            state,
            detail: None,
            cwd: None,
            title: None,
            pending: None,
            updated_at,
            acked_at: 0,
        }
    }

    /// 托管会话没了是比任何超时都硬的证据,必须排在时间判断之前 —— 哪怕它刚刚
    /// 才有过活动。
    #[test]
    fn dead_host_beats_every_timeout() {
        let s = session(AgentState::Working, 1000);
        assert_eq!(decide(1000, &s, false, 600, 300), Some(Sweep::Drop));
    }

    #[test]
    fn fresh_session_is_left_alone() {
        let s = session(AgentState::Working, 1000);
        assert_eq!(decide(1010, &s, true, 600, 300), None);
    }

    /// 人可能在睡觉。挂起请求自己有超时,轮不到清理来替它做决定。
    #[test]
    fn waiting_for_a_human_is_never_swept() {
        let mut s = session(AgentState::Waiting, 0);
        s.pending = Some("p1".into());
        assert_eq!(decide(99_999, &s, true, 600, 300), None);
    }

    #[test]
    fn stuck_working_becomes_idle() {
        let s = session(AgentState::Working, 0);
        assert_eq!(decide(301, &s, true, 600, 300), Some(Sweep::MarkIdle));
    }

    /// idle 的会话不该被「卡住」规则碰,它只等 session_stale。
    #[test]
    fn idle_session_is_not_marked_again() {
        let s = session(AgentState::Idle, 0);
        assert_eq!(decide(301, &s, true, 600, 300), None);
    }

    #[test]
    fn very_old_session_is_dropped() {
        let s = session(AgentState::Idle, 0);
        assert_eq!(decide(601, &s, true, 600, 300), Some(Sweep::Drop));
    }

    /// 用户点过「知道了」之后倒计时应当从那一刻重新开始。
    #[test]
    fn ack_extends_the_clock() {
        let mut s = session(AgentState::Idle, 0);
        s.acked_at = 600;
        assert_eq!(decide(900, &s, true, 600, 300), None);
        assert_eq!(decide(1201, &s, true, 600, 300), Some(Sweep::Drop));
    }

    /// `MarkIdle` 不刷新时间戳 —— 否则「超时转 idle」会把删除时刻不断往后推,
    /// 那条会话永远删不掉(参考实现踩过的坑)。
    #[test]
    fn mark_idle_does_not_extend_life() {
        let store = AgentSessionStore::new();
        store.apply(
            "claude-code",
            "s1",
            Some(1),
            Update { state: Some(AgentState::Working), ..Default::default() },
        );
        let before = store.list()[0].updated_at;
        let changed = store.sweep(before + 301, &|_| true, 600, 300);
        assert_eq!(changed.len(), 1);
        assert_eq!(store.list()[0].state, AgentState::Idle);
        assert_eq!(store.list()[0].updated_at, before, "MarkIdle 不该续命");
        // 再走到 session_stale,必须能被删掉
        let changed = store.sweep(before + 601, &|_| true, 600, 300);
        assert_eq!(changed.len(), 1);
        assert!(store.list().is_empty(), "转成 idle 之后仍然必须能被删掉");
    }

    #[test]
    fn sweep_drops_sessions_whose_host_is_gone() {
        let store = AgentSessionStore::new();
        store.apply("claude-code", "s1", Some(7), Update::default());
        let changed = store.sweep(now(), &|_| false, 600, 300);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].state, AgentState::Done);
        assert!(store.list().is_empty());
    }
}
