//! 设置窗口的状态与校验。这里不知道 egui 存在 —— 全部逻辑都能直接跑单测。

use std::net::{IpAddr, SocketAddr};

use crate::configfile::Editable;

/// 表单里存字符串而不是强类型:用户打字的过程中,绝大多数时刻内容都是非法的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub host: String,
    pub port: String,
    /// 空白表示「不固定」。
    pub token: String,
    pub autostart: bool,
}

/// 保存之后给用户看的一句话。定义在这里而不是 ui.rs:保存流程
/// ([`crate::settings::save`])要用它,而那一层刻意不认识 egui。
/// ui.rs 里有 `pub use`,对外仍是 `settings::ui::Message`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Ok(String),
    Err(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    Host(String),
    Port(String),
}

impl std::fmt::Display for FieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldError::Host(m) | FieldError::Port(m) => f.write_str(m),
        }
    }
}

impl Form {
    pub fn from_current(listen: SocketAddr, autostart: bool) -> Self {
        Self {
            host: listen.ip().to_string(),
            port: listen.port().to_string(),
            // token 不从当前值回填:它是密钥,默认展示为空(= 不固定)容易误伤,
            // 所以由调用方在构造后显式赋值,见 settings/ui.rs。
            token: String::new(),
            autostart,
        }
    }

    pub fn validate(&self) -> Result<Editable, FieldError> {
        let ip: IpAddr = self
            .host
            .trim()
            .parse()
            .map_err(|_| FieldError::Host(format!("不是合法的 IP 地址:{}", self.host.trim())))?;
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| FieldError::Port(format!("端口必须是 1-65535 的整数:{}", self.port.trim())))?;
        if port == 0 {
            // 0 会让系统随机分配端口,对一个要被访问的服务没有意义
            return Err(FieldError::Port("端口不能是 0".into()));
        }
        let token = self.token.trim();
        Ok(Editable {
            listen: SocketAddr::new(ip, port),
            token: (!token.is_empty()).then(|| token.to_string()),
        })
    }
}

/// 保存时要不要重启 HTTP 层。
///
/// 注意 `token: None`(不固定)**不**触发 rebind:它只是把键从 config.toml 拿掉,
/// 本次运行继续用现有 token,下次启动才随机 —— 否则保存一下就把自己踢下线了。
pub fn needs_rebind(current_listen: SocketAddr, current_token: &str, next: &Editable) -> bool {
    next.listen != current_listen
        || next.token.as_deref().is_some_and(|t| t != current_token)
}
