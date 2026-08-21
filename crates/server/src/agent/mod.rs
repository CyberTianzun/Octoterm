//! agent 集成。扩展点只有一个:[`AgentAdapter`]。
//!
//! 形态照抄 `launcher` 模块 —— 多来源、单一 trait、**失败局部化**:某个 adapter
//! 抛错只让它自己的条目消失并留一条日志,不影响其他 adapter,更不影响终端本身。
//!
//! 与 `launcher` 的关键区别,也是这个模块唯一需要小心的地方:launcher **只读**
//! 扫描别人的配置,agent 集成在用户显式动作下**会写**别人的配置。这条例外的边界是
//! 「发现只读、集成需显式动作」,由三道关卡守住:`agents.install_enabled` 开关、
//! 写前备份、卸载能还原。设计依据见
//! `docs/superpowers/specs/2026-08-18-octoterm-agent-integration-design.md`。

use serde::Serialize;
use std::path::PathBuf;

pub mod apply;
pub mod claude_code;
pub mod claude_transcript;
pub mod codex;
pub mod detect;
pub mod edit;
pub mod hook_cli;
pub mod routes;
pub mod store;
pub mod transcript;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// 「本机装没装这个 agent」的判定结果。
#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub installed: bool,
    pub confidence: Confidence,
    /// 机器可读的判定依据。客户端可以据此分组,但不该硬编码文案。
    pub reason: &'static str,
    /// 给人看的一句话,UI 直接显示,**不解析**。
    pub detail: String,
    pub config_path: Option<PathBuf>,
}

/// 我方集成当前处于什么状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Integration {
    NotInstalled,
    Installed,
    /// 装过,但 URL 指向的端口不是当前监听端口 —— 「装了却不生效」。
    ///
    /// 这是个必须单独报出来的状态:按实测,连不上的 hook 对 Claude 是
    /// non-blocking,它照常能用,于是这种失效**完全没有外部症状**,只有远程
    /// 接管会静默失灵。不主动报,用户永远不会发现。
    StalePort,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub detected: Detected,
    pub integration: Integration,
    /// 装完之后还需要用户做什么才生效。**机器可读的键**,不是文案 —— 文案由客户端
    /// 按自己的语言渲染(和 `detected.reason` 同一个路子)。
    ///
    /// 目前只有 Codex 有:它用 `trusted_hash` 逐条门控 hook,必须由用户在它自己的
    /// TUI 里 review 过才生效。这一步没法自动化,也**不该**自动化 —— 那道闸防的正是
    /// 「第三方悄悄让 Codex 执行任意命令」,而我们恰好就是那个第三方。
    pub activation: Option<&'static str>,
    /// 同一事件上发现的**别人的**阻塞式 hook,一行一条人类可读描述。
    ///
    /// 不是错误,也不会被我们删掉 —— 但两个阻塞 hook 抢同一个事件是真实的互操作
    /// 问题(例如本机同时装了 clawd-on-desk),必须让用户看得见。
    pub conflicts: Vec<String>,
}

pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;

    fn detect(&self, env: &detect::DetectEnv) -> Detected;

    /// 只产出「要对哪个文件做什么编辑」,**不写盘**。理由见 `edit` 模块的文档。
    fn plan_install(&self, ctx: &edit::InstallCtx) -> anyhow::Result<Vec<edit::ConfigEdit>>;
    fn plan_uninstall(&self, ctx: &edit::InstallCtx) -> anyhow::Result<Vec<edit::ConfigEdit>>;

    /// 只读地看一眼当前集成状态。读不到 / 读坏了一律当作没装。
    fn integration(&self, ctx: &edit::InstallCtx) -> (Integration, Vec<String>);

    /// 装完之后还需要用户做什么才生效。返回机器可读的键,`None` 表示装完即生效。
    fn activation(&self) -> Option<&'static str> {
        None
    }

    /// 把 agent 方言的 hook payload 归一化成统一更新。
    ///
    /// **不认识的事件返回 `None`,当作忽略而不是错误** —— agent 升级会带来新事件,
    /// 不能因为多了一个事件名就 500,更不能因此把 Claude 卡住。
    fn parse(&self, event: &str, body: &serde_json::Value) -> Option<store::Update>;

    /// 这家的对话记录能不能读。`false` 时聊天视图回落到终端,并说明是「还不支持」
    /// 而不是「出错了」—— 两者对用户是两回事。
    fn supports_transcript(&self) -> bool {
        false
    }

    /// 把这家的对话记录解析成归一化消息。只在 [`Self::supports_transcript`] 为真时调用。
    fn parse_transcript(&self, text: &str) -> Vec<transcript::Message> {
        let _ = text;
        Vec::new()
    }

    /// 这个事件是不是「阻塞式」的 —— agent 会挂在 socket 上等我们写响应。
    fn is_blocking(&self, event: &str) -> bool;

    /// 把统一决策渲染回 agent 方言的响应体。
    fn render(&self, decision: &store::Decision) -> serde_json::Value;
}

pub fn registry() -> Vec<Box<dyn AgentAdapter>> {
    vec![Box::new(claude_code::ClaudeCode), Box::new(codex::Codex)]
}

pub fn find(id: &str) -> Option<Box<dyn AgentAdapter>> {
    registry().into_iter().find(|a| a.id() == id)
}

/// 扫描所有 adapter。单个 adapter 的失败只影响它自己那一条(局部失败原则)。
pub fn scan(env: &detect::DetectEnv, port: u16) -> Vec<AgentStatus> {
    let ctx = edit::InstallCtx { home: env.home.clone(), port, include_blocking: true };
    registry()
        .into_iter()
        .map(|a| {
            let (integration, conflicts) = a.integration(&ctx);
            AgentStatus {
                id: a.id(),
                name: a.name(),
                detected: a.detect(env),
                integration,
                activation: a.activation(),
                conflicts,
            }
        })
        .collect()
}
