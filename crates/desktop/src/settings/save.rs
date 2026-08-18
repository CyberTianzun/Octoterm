//! 「保存并应用」的执行顺序与失败语义。
//!
//! 从 app.rs 里抽出来是为了能测:这是整个设置界面唯一有实质逻辑的部分(校验失败 /
//! 不需要 rebind / rebind 成功 / rebind 后写文件失败 / rebind 失败,五个分支各有
//! 各的提示与善后),而 app.rs 里全是 winit 事件循环和 tokio runtime,搬不进单测。
//! 所有 IO 通过 [`Effects`] 注入,测试给一个假的实现就能把五个分支跑一遍。
//!
//! 顺序是「先 restart、再写 config.toml」。设计文档写的是「先 bind、再写文件、再
//! 切换」,但 [`crate::supervisor::Supervisor::restart`] 已经把 bind 与切换封成了
//! 一步(地址有变化时它内部保证先 bind 后关),所以这里只剩两步,失败语义不变:
//! restart 失败时什么都没动;写文件失败时如实告诉用户「跑起来了但没存下」。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::configfile::Editable;
use crate::settings::state::{needs_rebind, Form, Message};

/// 保存要做的全部 IO。真实实现在 app.rs(改开机自启、找配置路径、写文件、
/// 重启 HTTP 层),测试里换成记账用的假实现。
pub trait Effects {
    fn set_autostart(&mut self, enabled: bool) -> Result<()>;
    fn config_path(&mut self) -> Result<PathBuf>;
    fn save_config(&mut self, path: &Path, edit: &Editable) -> Result<()>;
    /// 返回实际监听到的地址。
    fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr>;
}

/// 点下「保存并应用」那一刻的现状快照。
#[derive(Debug, Clone)]
pub struct Current {
    /// `None` = 当前没在监听(比如启动时端口就被占着)。
    pub listen: Option<SocketAddr>,
    /// 当前生效的 token;没在监听时是空串。
    pub token: String,
    /// 当前会话数,只用来拼提示文案。
    pub sessions: usize,
}

/// 保存的结果:给用户看的一句话,外加调用方还需不需要善后。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub message: Message,
    /// HTTP 层确实重启过 —— 调用方要刷新托盘状态行(监听地址可能变了)。
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
    // 后面再报「已保存」就是在撒谎。
    if let Err(e) = fx.set_autostart(form.autostart) {
        return Applied::err(format!("开机自启设置失败:{e}"));
    }

    let path = match fx.config_path() {
        Ok(p) => p,
        Err(e) => return Applied::err(e.to_string()),
    };

    let rebind = match current.listen {
        Some(listen) => needs_rebind(listen, &current.token, &next),
        None => true, // 当前没在监听,无论如何都要试着起来
    };

    if !rebind {
        // 只改了 token 的固定与否 / 只改了自启:写文件就完事
        return match fx.save_config(&path, &next) {
            Ok(()) => Applied { message: Message::Ok("已保存".into()), restarted: false },
            Err(e) => Applied::err(e.to_string()),
        };
    }

    // token 留空(不固定)时,本次运行继续用现有的 —— 只是不再写进 config.toml。
    let token = next.token.clone().unwrap_or_else(|| current.token.clone());
    match fx.restart(next.listen, token) {
        Ok(actual) => {
            let message = match fx.save_config(&path, &next) {
                Ok(()) => Message::Ok(format!(
                    "已生效 · {actual} · {} 个会话未受影响",
                    current.sessions
                )),
                // 已经在新地址上跑起来了,但配置没落盘 —— 必须说清楚
                Err(e) => Message::Err(format!(
                    "已在 {actual} 生效,但写入配置失败:{e}(重启后会回到旧配置)"
                )),
            };
            Applied { message, restarted: true }
        }
        // restart 失败:supervisor 保证地址有变化时旧的还在跑,这里什么都没动。
        Err(e) => Applied::err(e.to_string()),
    }
}
