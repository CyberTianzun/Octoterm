use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

fn default_listen() -> SocketAddr {
    "127.0.0.1:7683".parse().unwrap()
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

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub window_size: WindowSize,
    /// TOML 里是 `[[launcher]]`(单数),读出来是一组。
    #[serde(default, rename = "launcher")]
    pub launchers: Vec<LauncherSpec>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            token: None,
            window_size: WindowSize::default(),
            launchers: Vec::new(),
        }
    }
}

fn default_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "octoterm")
        .context("cannot determine config directory")?;
    Ok(dirs.config_dir().join("config.toml"))
}

impl Config {
    /// 只读加载,server 自己永不写文件(写是 octoterm-desktop 的职责):
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
