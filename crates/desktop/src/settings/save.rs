//! 「保存并应用」的执行顺序与失败语义。
//!
//! 从 app.rs 里抽出来是为了能测:这是整个设置界面唯一有实质逻辑的部分(校验失败 /
//! 不需要 rebind / rebind 成功 / rebind 后写文件失败 / rebind 失败,五个分支各有
//! 各的提示与善后),而 app.rs 里全是 winit 事件循环和 tokio runtime,搬不进单测。
//! 所有 IO 通过 [`Effects`] 注入,测试给一个假的实现就能把五个分支跑一遍。
//!
//! 顺序是「先 restart、再写 config.toml」。设计文档写的是「先 bind、再写文件、再
//! 切换」,但 [`crate::supervisor::Supervisor::restart`] 已经把 bind 与切换封成了
//! 一步(地址有变化时它内部保证先 bind 后关),所以这里只剩两步。
//!
//! 失败之后各步的善后**不一样**,不能一句「什么都没动」带过:
//!
//! - 校验失败:一个 IO 都没做。
//! - 校验通过之后 **开机自启是第一个被改的**,后面任何一步失败它都已经生效了
//!   (这是有意的:它和 HTTP 层无关,没有理由陪着一起回滚)。
//! - `restart` 失败:配置文件肯定没写。但 HTTP 层的状态取决于地址有没有变 ——
//!   地址有变化时旧的还在跑;地址没变(只换 token)时 supervisor 已经先 `stop()`
//!   了,失败就停在「完全没有 HTTP 层」上。提示文案里必须把这一点说出来,调用方
//!   也必须无条件刷新状态行。
//! - 写文件失败:HTTP 层已经在新地址上跑起来了,如实告诉用户「跑起来了但没存下」。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::configfile::Editable;
use crate::settings::state::{needs_rebind, Form, Message};

/// 保存要做的全部 IO。真实实现在 app.rs(改开机自启、找配置路径、写文件、
/// 重启 HTTP 层、生成新 token),测试里换成记账用的假实现。
pub trait Effects {
    fn set_autostart(&mut self, enabled: bool) -> Result<()>;
    fn config_path(&mut self) -> Result<PathBuf>;
    fn save_config(&mut self, path: &Path, edit: &Editable) -> Result<()>;
    /// 返回实际监听到的地址。
    fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr>;
    /// 现场生成一个新的随机 token。格式与 `octoterm_server::config::resolve_token`
    /// 一致(32 位无连字符十六进制)。**不允许返回空串。**
    fn new_token(&mut self) -> String;
}

/// 点下「保存并应用」那一刻的现状快照。
#[derive(Debug, Clone)]
pub struct Current {
    /// `None` = 当前没在监听(比如启动时端口就被占着)。
    pub listen: Option<SocketAddr>,
    /// 当前生效的 token。`None` = 当前没在监听,压根不存在「生效的 token」。
    ///
    /// 刻意用 `Option` 而不是空串:空串是一个**合法但灾难性**的 token —— server
    /// 侧鉴权是 `token == state.token` 的直接比较,两边都空就一律放行。用空串
    /// 冒充「没有」,迟早会被当成一个真 token 送进 [`Effects::restart`]。
    pub token: Option<String>,
    /// 当前会话数,只用来拼提示文案。
    pub sessions: usize,
}

/// 保存的结果:给用户看的一句话,外加调用方还需不需要善后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub message: Message,
    /// HTTP 层确实重启成功、并且在(可能是新的)地址上跑起来了。
    ///
    /// **不要拿它当「要不要刷新状态行」的开关**:`restart` 失败时它是 `false`,
    /// 而同地址那条失败路径恰恰会让服务停在未监听状态 —— 状态行更需要刷新。
    /// 调用方无条件刷新即可,这个字段只是给测试和日志用的事实描述。
    pub restarted: bool,
}

impl Applied {
    fn err(text: String) -> Self {
        Self { message: Message::Err(text), restarted: false }
    }
}

pub fn apply(form: &Form, current: &Current, fx: &mut impl Effects) -> Applied {
    let next = match form.validate() {
        Ok(v) => v,
        Err(e) => return Applied::err(e.to_string()),
    };

    // 开机自启和 server 无关,单独处理。它失败就直接停下:用户勾的这一项没生效,
    // 后面再报「已保存」就是在撒谎。反过来它**成功**之后就不再回滚了 —— 后面
    // 任何一步失败,这一项都已经改掉了。
    if let Err(e) = fx.set_autostart(form.autostart) {
        return Applied::err(format!("开机自启设置失败:{e:#}"));
    }

    let path = match fx.config_path() {
        Ok(p) => p,
        Err(e) => return Applied::err(format!("{e:#}")),
    };

    // 注意这里的 `!token.trim().is_empty()`:**空串不是 token**,当前正带着空
    // token 在跑,和「当前没在监听」一样要强制重建。少了这个条件,
    // `needs_rebind(listen, "", &Editable { token: None })` 会算出 false,于是
    // 「服务带着空 token 在跑 + 表单 token 留空 + 地址没改」这一组会从下面
    // `if !rebind` 那条提前返回直接溜走 —— 底下的三级回退一行都跑不到,空 token
    // 的 HTTP 层原封不动继续对所有人放行,用户看到的却是绿字「已保存」。
    let rebind = match (current.listen, current.token.as_deref()) {
        (Some(listen), Some(token)) if !token.trim().is_empty() => {
            needs_rebind(listen, token, &next)
        }
        // 当前没在监听、或当前 token 是空的:无论如何都要(重新)起来
        _ => true,
    };

    if !rebind {
        // 只改了 token 的固定与否 / 只改了自启:写文件就完事
        return match fx.save_config(&path, &next) {
            Ok(()) => Applied { message: Message::Ok("已保存".into()), restarted: false },
            Err(e) => Applied::err(format!("{e:#}")),
        };
    }

    // 送进 restart 的 token **必须非空**。server 侧鉴权(`bearer_ok` 与 WebSocket
    // 握手)是 `token == state.token` 的直接比较,空 token 会让任何人凭
    // `Authorization: Bearer ` 或 `Hello{token:""}` 进来 —— 等于整个鉴权关掉。
    // 三级回退:
    //   1. 表单填了(validate 保证非空)→ 用它;
    //   2. 表单留空、当前有在跑 → 沿用现行 token(不然保存一下自己就被踢下线),
    //      只是不再写进 config.toml;
    //   3. 表单留空、当前压根没在监听(没有现行 token)→ 现场生成一个新的。
    let token = match next.token.clone() {
        Some(t) => t,
        None => match current.token.clone() {
            Some(t) if !t.trim().is_empty() => t,
            _ => fx.new_token(),
        },
    };
    // 本地的一道保险,不是最后一道:真正的收口在
    // [`crate::supervisor::Supervisor::restart`] 的 `ensure!` —— 那里 release 构建
    // 也生效,而且是所有 token 进入 `AppState` 的必经之路。这里留一条断言只是为了
    // 在单测(用假的 `Effects`,够不着 supervisor)里就地炸掉。
    debug_assert!(!token.trim().is_empty(), "空 token 会让 server 侧鉴权全部放行");

    match fx.restart(next.listen, token) {
        Ok(actual) => {
            let message = match fx.save_config(&path, &next) {
                Ok(()) => Message::Ok(format!(
                    "已生效 · {actual} · {} 个会话未受影响",
                    current.sessions
                )),
                // 已经在新地址上跑起来了,但配置没落盘 —— 必须说清楚
                Err(e) => Message::Err(format!(
                    "已在 {actual} 生效,但写入配置失败:{e:#}(重启后会回到旧配置)"
                )),
            };
            Applied { message, restarted: true }
        }
        // restart 失败:配置文件没写,但 HTTP 层的现状要看走的是哪条路径,
        // 见 `Supervisor::restart` 的文档。用户只看到一句 bind 失败的话,是判断
        // 不出服务到底还在不在的。
        Err(e) => Applied::err(match current.listen {
            // 地址有变化:supervisor 先 bind 新的、成功了才关旧的,旧的还在跑
            Some(old) if old != next.listen => format!("{e:#}(原服务仍在 {old} 上运行)"),
            // 地址没变(只换 token):supervisor 已经先停了旧的,再失败就没有
            // HTTP 层了;current.listen 为 None 时本来就没在监听
            _ => format!("{e:#}(服务当前未监听)"),
        }),
    }
}
