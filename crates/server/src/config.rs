use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// `octoterm-server` 的默认监听:**只回环**。
///
/// 无头部署往往跑在别人的机器上、被别的东西转发,默认对外开是不礼貌也不安全的。
/// 要对外用 `--host` 显式说出来。
pub fn default_listen() -> SocketAddr {
    "127.0.0.1:7683".parse().unwrap()
}

/// `octoterm-desktop` 的默认监听:**全网卡**。
///
/// 和 server 不同是有意的。desktop 的存在理由就是「从手机/平板连回自己这台机器」——
/// 默认只回环等于这个产品开箱即不可用,而绝大多数人不会去翻文档改配置。
///
/// 敢这么定的前提是两条硬边界都已经在了:
///
/// 1. **空 token 进不去**。desktop 里所有 token 都经过 `Supervisor::restart` 开头那个
///    `ensure!`(release 构建也拦),所以「全网卡 + 无鉴权」在这条路径上结构性不可能;
/// 2. **hook 面无条件只认回环**。`/hook/*` 看的是对端地址,主监听是不是 0.0.0.0 都一样
///    —— 那条路上跑的是 `tool_input`(命令原文、文件路径),不对外开。
///
/// 剩下的残余风险是实打实的:同一局域网里的人只要拿到 token 就能用你的终端。
/// 这和 README 里说的「越过 localhost 就自己带网络层安全」是同一件事,只是现在
/// 默认值站在了「能用」这一边。用户可以在设置窗口里改回 127.0.0.1。
pub fn desktop_default_listen() -> SocketAddr {
    "0.0.0.0:7683".parse().unwrap()
}

/// 多个连接同时 attach 同一个会话时,pty 用谁的尺寸(tmux `window-size` 的语义)。
///
/// 一个会话只有一个 pty、一份 grid,所有 attach 收到的是同一份字节流,服务端
/// 无法逐客户端裁剪画面,所以必须从各端的尺寸诉求里归并出一个权威值。这是服务端
/// 策略,不在协议里(G3):客户端只上报自己想要多大,再按 `resized` 渲染。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum WindowSize {
    /// 取所有 attach 的最小值:谁都不会看到被截断的画面,大屏那端留白。
    #[default]
    Smallest,
    /// 取最大值:小屏那端只能看到画面的一部分。
    Largest,
    /// 跟随最近一次 attach/resize 的那一端。
    Latest,
}

/// 用户在 config.toml 里写的一条自定义启动项(`[[launcher]]`)。
///
/// `command` 直接就是 argv,不是命令行字符串 —— 省掉了"该按哪套规则切分"这个
/// 问题(见 launcher/cmdline.rs 里两套互不兼容的规则)。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LauncherSpec {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

/// `[agents]` —— agent 集成。
///
/// `install_enabled` **默认关**:装 hook 会改用户的 `~/.claude/settings.json`,
/// 并且改变这台机器上所有 Claude 会话的行为。这种事必须是显式选择,不能因为
/// 「跑起来了」就自动发生。headless / 共享部署可以把它永久关掉。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub install_enabled: bool,
    #[serde(default = "default_session_stale")]
    pub session_stale_secs: u64,
    #[serde(default = "default_working_stale")]
    pub working_stale_secs: u64,
    /// 一个挂起请求最多等多久。
    ///
    /// 必须**小于**写进 hook 里的 `timeout`(600 秒),这样超时是我们主动写一个
    /// 「无决定」的响应,而不是让 Claude 那头自己超时 —— 两者行为一样,但前者
    /// 我们知道发生了什么,能记日志、能把状态改回去。
    #[serde(default = "default_pending_timeout")]
    pub pending_timeout_secs: u64,
}

fn default_pending_timeout() -> u64 {
    590
}

fn default_session_stale() -> u64 {
    600
}

fn default_working_stale() -> u64 {
    300
}

impl Default for AgentsConfig {
    fn default() -> Self {
        Self {
            install_enabled: false,
            session_stale_secs: default_session_stale(),
            working_stale_secs: default_working_stale(),
            pending_timeout_secs: default_pending_timeout(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    /// 配置文件里没写就是 `None` —— **「没指定」是个真实存在的状态**,不能在这一层
    /// 塞一个默认值把它抹平:server 和 desktop 的默认监听本来就不一样,谁用谁定。
    #[serde(default)]
    pub listen: Option<SocketAddr>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub window_size: WindowSize,
    /// TOML 里是 `[[launcher]]`(单数),读出来是一组。
    #[serde(default, rename = "launcher")]
    pub launchers: Vec<LauncherSpec>,
    #[serde(default)]
    pub agents: AgentsConfig,
}


fn default_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "octoterm")
        .context("cannot determine config directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

impl Config {
    /// 只读加载:**自己的** config.toml 由 octoterm-desktop 写,server 只读。
    /// (server 唯一会写的是 agent 集成装 hook 时改的**别人的**配置文件,受
    /// `agents.install_enabled` 门控,见 `crate::agent::apply`。)
    /// 显式路径必须存在;缺省路径存在则读,不存在用默认值。
    pub fn load(path: Option<PathBuf>) -> Result<Config> {
        let path = match path {
            Some(p) => {
                if !p.exists() {
                    bail!("config file not found: {}", p.display());
                }
                p
            }
            None => {
                let p = default_path()?;
                if !p.exists() {
                    return Ok(Config::default());
                }
                p
            }
        };
        Ok(toml::from_str(&std::fs::read_to_string(&path)?)?)
    }
}

/// token 优先级:--token > 配置文件 > 每次启动新生成(Jupyter 式)。
/// 返回 (token, 是否本次生成)。
///
/// **空白值等于没配**:`""` 和 `"   "` 一律当作 `None` 处理,顺着优先级往下走
/// (`--token ""` 因此也不会盖掉配置文件里的好 token),最终落到随机生成那条路。
/// 这是鉴权底线 —— 鉴权是 `token == state.token` 的字面比较(见 app.rs 的
/// WebSocket 握手),空 token 生效就等于「空对空一律放行」,配上 `--host 0.0.0.0`
/// 就是一个全网卡、无鉴权的终端服务。而空值恰恰是最容易写出来的:设置界面写着
/// 「留空表示不固定」,配置文件里 `token = ""` 看着也像「不设 token」。
///
/// **前后空白会被去掉**:`token = "  abc  "` 的生效值是 `"abc"`。原样保留的话,
/// 客户端几乎不可能把两端的空格一字不差地发上来,结果是个谁也连不上的 server。
pub fn resolve_token(cli: Option<String>, config: Option<String>) -> (String, bool) {
    let pick = |v: Option<String>| v.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    match pick(cli).or_else(|| pick(config)) {
        Some(t) => (t, false),
        None => (uuid::Uuid::new_v4().simple().to_string(), true),
    }
}

/// 命令行 --host/--port 覆盖配置文件的 listen;None 表示沿用配置值
pub fn effective_listen(
    base: SocketAddr,
    host: Option<std::net::IpAddr>,
    port: Option<u16>,
) -> SocketAddr {
    let mut listen = base;
    if let Some(host) = host {
        listen.set_ip(host);
    }
    if let Some(port) = port {
        listen.set_port(port);
    }
    listen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> SocketAddr {
        "127.0.0.1:7683".parse().unwrap()
    }

    #[test]
    fn no_overrides_keeps_config() {
        assert_eq!(effective_listen(base(), None, None), base());
    }

    /// 「配置文件里没写 listen」必须能和「写了 127.0.0.1」区分开 —— 两个二进制的
    /// 默认值不一样,在 Config 这一层塞默认值就把这个区别抹平了。
    #[test]
    fn absent_listen_stays_absent() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.listen, None);
        let cfg: Config = toml::from_str(r#"listen = "127.0.0.1:1234""#).unwrap();
        assert_eq!(cfg.listen, Some("127.0.0.1:1234".parse().unwrap()));
    }

    /// server 只回环、desktop 全网卡,这是刻意的差异,不是笔误。
    #[test]
    fn the_two_binaries_default_differently() {
        assert!(default_listen().ip().is_loopback());
        assert!(desktop_default_listen().ip().is_unspecified());
        assert_eq!(default_listen().port(), desktop_default_listen().port());
    }

    #[test]
    fn host_and_port_override_independently() {
        assert_eq!(
            effective_listen(base(), Some("0.0.0.0".parse().unwrap()), None).to_string(),
            "0.0.0.0:7683"
        );
        assert_eq!(
            effective_listen(base(), None, Some(9000)).to_string(),
            "127.0.0.1:9000"
        );
    }

    #[test]
    fn both_overrides_apply() {
        assert_eq!(
            effective_listen(base(), Some("::1".parse().unwrap()), Some(9000)).to_string(),
            "[::1]:9000"
        );
    }

    #[test]
    fn window_size_defaults_to_smallest_and_parses_kebab_case() {
        assert_eq!(Config::default().window_size, WindowSize::Smallest);
        let cfg: Config = toml::from_str("window_size = \"latest\"").unwrap();
        assert_eq!(cfg.window_size, WindowSize::Latest);
    }

    #[test]
    fn launchers_parse_from_double_bracket_tables() {
        let cfg: Config = toml::from_str(
            r#"
            [[launcher]]
            name = "prod ssh"
            command = ["ssh", "prod01"]
            cwd = "~/work"

            [[launcher]]
            name = "python"
            command = ["python3", "-i"]
            "#,
        )
        .unwrap();
        assert_eq!(cfg.launchers.len(), 2);
        assert_eq!(cfg.launchers[0].command, ["ssh", "prod01"]);
        assert_eq!(cfg.launchers[0].cwd.as_deref(), Some("~/work"));
        assert_eq!(cfg.launchers[1].cwd, None);
    }

    #[test]
    fn missing_launcher_section_yields_an_empty_list() {
        let cfg: Config = toml::from_str("").unwrap();
        assert!(cfg.launchers.is_empty());
    }

    #[test]
    fn token_priority_cli_then_config_then_generated() {
        assert_eq!(
            resolve_token(Some("cli".into()), Some("cfg".into())),
            ("cli".into(), false)
        );
        assert_eq!(resolve_token(None, Some("cfg".into())), ("cfg".into(), false));
        let (t1, generated) = resolve_token(None, None);
        assert!(generated && !t1.is_empty());
        let (t2, _) = resolve_token(None, None);
        assert_ne!(t1, t2, "每次生成的 token 必须不同");
    }

    #[test]
    fn blank_cli_token_does_not_shadow_config() {
        // `--token ""` 不是「把 token 设成空」,而是「没给」—— 否则它会盖掉配置
        // 文件里那个好 token,把 server 变成空对空放行。
        assert_eq!(
            resolve_token(Some(String::new()), Some("cfg".into())),
            ("cfg".into(), false)
        );
        assert_eq!(
            resolve_token(Some("   ".into()), Some("cfg".into())),
            ("cfg".into(), false)
        );
    }

    #[test]
    fn blank_token_falls_back_to_generated() {
        // 配置文件里 `token = ""` / `token = "   "` 等同于没配,走随机生成。
        for blank in ["", "   ", "\t\n"] {
            let (t, generated) = resolve_token(None, Some(blank.into()));
            assert!(generated, "空白 token 必须回退到随机生成:{blank:?}");
            assert!(!t.trim().is_empty());
        }
        let (t, generated) = resolve_token(Some("  ".into()), None);
        assert!(generated && !t.trim().is_empty());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        // 生效值必须是 trim 过的:鉴权是字面比较,带空格的 token 客户端发不对。
        assert_eq!(
            resolve_token(None, Some("  abc  ".into())),
            ("abc".into(), false)
        );
        assert_eq!(
            resolve_token(Some("\tcli\n".into()), Some("cfg".into())),
            ("cli".into(), false)
        );
    }
}
