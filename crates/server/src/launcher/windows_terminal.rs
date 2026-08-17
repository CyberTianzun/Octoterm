//! Windows Terminal 的 profile。
//!
//! 只读 `settings.json`,不写。解析([`parse`])是平台无关的纯函数,路径发现
//! ([`settings_paths`])才按平台 gate —— 否则这套逻辑只能在 Windows 上测,而
//! 它恰恰是最容易被 schema 变动咬到的地方。

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use super::{cmdline, jsonc, Launcher, LauncherProvider};

pub const ID: &str = "windows-terminal";

/// WSL 的 profile 由 WT 的动态生成器产出,配置里**没有 commandline**,只有一个
/// `source` 标记和发行版名字。这是唯一一个值得我们代劳合成命令的情形:它太常见
/// 了,而且规则是稳定的。其余没有 commandline 的动态 profile(PowerShell Core、
/// Azure Cloud Shell 等)命令行无从推断,直接跳过,不猜。
const WSL_SOURCE: &str = "Windows.Terminal.Wsl";

pub struct WindowsTerminal;

impl WindowsTerminal {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl LauncherProvider for WindowsTerminal {
    fn id(&self) -> &'static str {
        ID
    }

    fn discover(&self) -> Result<Vec<Launcher>> {
        let env = |k: &str| std::env::var(k).ok();
        for path in settings_paths() {
            // 没装 / 没这个版本:不是错误,换下一个候选
            let Ok(src) = std::fs::read_to_string(&path) else { continue };
            tracing::debug!(path = %path.display(), "读取 Windows Terminal 配置");
            return parse(&src, &env);
        }
        Ok(Vec::new())
    }
}

/// settings.json 的候选位置,按优先级。存在多个时只用第一个命中的。
#[cfg(windows)]
pub fn settings_paths() -> Vec<PathBuf> {
    let Ok(local) = std::env::var("LOCALAPPDATA") else { return Vec::new() };
    let local = PathBuf::from(local);
    vec![
        local.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"),
        local
            .join(r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json"),
        // 非应用商店(portable / MSI)安装
        local.join(r"Microsoft\Windows Terminal\settings.json"),
    ]
}

#[cfg(not(windows))]
pub fn settings_paths() -> Vec<PathBuf> {
    Vec::new()
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// 解析 settings.json 的内容。`env` 用来展开 `%VAR%`,注入进来是为了可测。
pub fn parse(src: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<Vec<Launcher>> {
    let root: Value = serde_json::from_str(&jsonc::strip(src))?;

    // `profiles` 现在是 `{ defaults, list }`,早期版本直接是个数组,两种都认
    let profiles = root.get("profiles");
    let (defaults, list) = match profiles {
        Some(Value::Array(list)) => (None, list.clone()),
        Some(obj @ Value::Object(_)) => (
            obj.get("defaults").cloned(),
            obj.get("list").and_then(Value::as_array).cloned().unwrap_or_default(),
        ),
        _ => (None, Vec::new()),
    };
    let default_of = |key: &str| defaults.as_ref().and_then(|d| str_field(d, key).map(str::to_string));

    let mut out = Vec::new();
    for p in &list {
        if p.get("hidden").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let Some(name) = str_field(p, "name") else { continue };
        let raw = str_field(p, "commandline").map(str::to_string).or_else(|| default_of("commandline"));
        let source = str_field(p, "source");

        let (command, detail) = match (&raw, source) {
            (Some(raw), _) => {
                let argv = cmdline::split_windows(&cmdline::expand_windows_env(raw, env));
                if argv.is_empty() {
                    continue;
                }
                (argv, raw.clone())
            }
            (None, Some(WSL_SOURCE)) => {
                let argv = vec!["wsl.exe".to_string(), "-d".to_string(), name.to_string()];
                let detail = argv.join(" ");
                (argv, detail)
            }
            (None, _) => continue,
        };

        let cwd = str_field(p, "startingDirectory")
            .map(str::to_string)
            .or_else(|| default_of("startingDirectory"))
            .map(|d| cmdline::expand_windows_env(&d, env));

        // guid 是 WT 里真正稳定的键;缺了才退回名字
        let local_id = str_field(p, "guid").unwrap_or(name);
        out.push(
            Launcher::new(ID, local_id, name, command).with_detail(detail).with_cwd(cwd),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<String> {
        match k {
            "SystemRoot" => Some(r"C:\Windows".into()),
            "USERPROFILE" => Some(r"C:\Users\hiro".into()),
            _ => None,
        }
    }

    const SAMPLE: &str = r#"
    {
        // Windows Terminal 自己生成的配置就带注释
        "defaultProfile": "{guid-ps}",
        "profiles":
        {
            "defaults": { "startingDirectory": "%USERPROFILE%" },
            "list":
            [
                {
                    "guid": "{guid-cmd}",
                    "name": "命令提示符",
                    "commandline": "%SystemRoot%\\System32\\cmd.exe",
                    "startingDirectory": "C:\\work",
                },
                {
                    "guid": "{guid-ps7}",
                    "name": "PowerShell 7",
                    "commandline": "\"C:\\Program Files\\PowerShell\\7\\pwsh.exe\" -NoLogo"
                },
                {
                    "guid": "{guid-wsl}",
                    "name": "Ubuntu",
                    "source": "Windows.Terminal.Wsl"
                },
                {
                    "guid": "{guid-hidden}",
                    "name": "藏起来的",
                    "commandline": "cmd.exe",
                    "hidden": true
                },
                {
                    "guid": "{guid-azure}",
                    "name": "Azure Cloud Shell",
                    "source": "Windows.Terminal.Azure"
                }
            ]
        }
    }"#;

    #[test]
    fn parses_profiles_with_env_and_quoting() {
        let out = parse(SAMPLE, &env).unwrap();
        let ids: Vec<&str> = out.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            ["windows-terminal:{guid-cmd}", "windows-terminal:{guid-ps7}", "windows-terminal:{guid-wsl}"],
            "hidden 的和无法推断命令的应该被跳过"
        );

        let cmd = &out[0];
        assert_eq!(cmd.name, "命令提示符");
        assert_eq!(cmd.command, [r"C:\Windows\System32\cmd.exe"]);
        assert_eq!(cmd.cwd.as_deref(), Some(r"C:\work"));
        // detail 保留原文,用户在 WT 里看到的就是这个
        assert_eq!(cmd.detail, r"%SystemRoot%\System32\cmd.exe");

        // 带空格的程序路径不能被拆开
        assert_eq!(
            out[1].command,
            [r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo"]
        );
        // defaults 的 startingDirectory 在自己没写时生效
        assert_eq!(out[1].cwd.as_deref(), Some(r"C:\Users\hiro"));
    }

    #[test]
    fn wsl_profiles_get_a_synthesized_command() {
        let out = parse(SAMPLE, &env).unwrap();
        let wsl = out.iter().find(|l| l.name == "Ubuntu").unwrap();
        assert_eq!(wsl.command, ["wsl.exe", "-d", "Ubuntu"]);
    }

    #[test]
    fn accepts_the_legacy_array_shaped_profiles_key() {
        let out = parse(
            r#"{ "profiles": [ { "guid": "{g}", "name": "老格式", "commandline": "cmd.exe" } ] }"#,
            &env,
        )
        .unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].command, ["cmd.exe"]);
    }

    #[test]
    fn missing_or_empty_profiles_is_not_an_error() {
        assert!(parse("{}", &env).unwrap().is_empty());
        assert!(parse(r#"{"profiles": {"list": []}}"#, &env).unwrap().is_empty());
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse("{ not json", &env).is_err());
    }
}
