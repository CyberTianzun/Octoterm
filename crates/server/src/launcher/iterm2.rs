//! iTerm2 的 profile(iTerm2 内部叫 bookmark)。
//!
//! 两个来源:主 plist 里的 `New Bookmarks`,以及 `DynamicProfiles/` 目录下的
//! JSON 文件。两边的键名完全一样,只是容器格式不同,所以先各自抽成 [`Raw`],
//! 后面的判断只写一遍。
//!
//! **只收命令或工作目录跟默认不同的 profile。** iTerm2 用户常有一堆只改了配色
//! 的 profile,把它们全塞进"新建会话"菜单只是噪音 —— 在 octoterm 里配色是
//! 客户端的事(见设置面板),这个菜单只回答"跑什么、在哪跑"。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::{builtin, cmdline, Launcher, LauncherProvider};

pub const ID: &str = "iterm2";

pub struct ITerm2;

impl ITerm2 {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ITerm2 {
    fn default() -> Self {
        Self::new()
    }
}

impl LauncherProvider for ITerm2 {
    fn id(&self) -> &'static str {
        ID
    }

    fn discover(&self) -> Result<Vec<Launcher>> {
        let home = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf());
        let default = builtin::default_command();
        let mut raws = Vec::new();

        if let Some(path) = prefs_path(home.as_deref()) {
            if path.is_file() {
                tracing::debug!(path = %path.display(), "读取 iTerm2 配置");
                let value = plist::Value::from_file(&path)
                    .with_context(|| format!("解析 {} 失败", path.display()))?;
                raws.extend(from_prefs_plist(&value));
            }
        }
        for path in dynamic_profile_files(home.as_deref()) {
            match std::fs::read_to_string(&path) {
                // 单个动态 profile 文件坏了不该让整个 provider 归零
                Ok(src) => match serde_json::from_str::<serde_json::Value>(&src) {
                    Ok(v) => raws.extend(from_dynamic_json(&v)),
                    Err(e) => tracing::warn!(path = %path.display(), error = %e, "动态 profile 解析失败"),
                },
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "动态 profile 读取失败"),
            }
        }
        Ok(build(&raws, &default, home.as_deref()))
    }
}

/// 主 plist 的位置。iTerm2 允许把偏好设置放到自定义目录(「Load preferences from
/// a custom folder」),开了这个开关的用户,标准位置那份是过期的。
///
/// 注意:这里读的是磁盘上的文件,而 macOS 的 cfprefsd 会缓存写入 —— 刚在 iTerm2
/// 里改完设置、它还没落盘时,这里可能读到旧值。不做处理:重开一次菜单就好了。
#[cfg(target_os = "macos")]
pub fn prefs_path(home: Option<&Path>) -> Option<PathBuf> {
    const FILE: &str = "com.googlecode.iterm2.plist";
    let standard = home?.join("Library/Preferences").join(FILE);
    let custom = plist::Value::from_file(&standard).ok().and_then(|v| {
        let dict = v.into_dictionary()?;
        let enabled = dict.get("LoadPrefsFromCustomFolder").and_then(|v| v.as_boolean()).unwrap_or(false);
        let folder = dict.get("PrefsCustomFolder").and_then(|v| v.as_string())?.to_string();
        if !enabled || folder.trim().is_empty() {
            return None;
        }
        Some(PathBuf::from(cmdline::expand_tilde(&folder, home)).join(FILE))
    });
    Some(custom.filter(|p| p.is_file()).unwrap_or(standard))
}

#[cfg(not(target_os = "macos"))]
pub fn prefs_path(_home: Option<&Path>) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
pub fn dynamic_profile_files(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else { return Vec::new() };
    let dir = home.join("Library/Application Support/iTerm2/DynamicProfiles");
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> =
        entries.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect();
    files.sort(); // 目录序不稳定,排一下,菜单顺序才是确定的
    files
}

#[cfg(not(target_os = "macos"))]
pub fn dynamic_profile_files(_home: Option<&Path>) -> Vec<PathBuf> {
    Vec::new()
}

/// 一个 profile 里我们关心的那几个键,已经从 plist / JSON 里抽出来。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Raw {
    pub guid: Option<String>,
    pub name: Option<String>,
    /// `"No"` | `"Yes"` | `"Custom Shell"`
    pub custom_command: Option<String>,
    pub command: Option<String>,
    /// `"No"` | `"Yes"` | `"Recycle"` | `"Advanced"`
    pub custom_directory: Option<String>,
    pub working_directory: Option<String>,
}

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn from_prefs_plist(value: &plist::Value) -> Vec<Raw> {
    let Some(dict) = value.as_dictionary() else { return Vec::new() };
    let Some(list) = dict.get("New Bookmarks").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|p| {
            let d = p.as_dictionary()?;
            let get = |k: &str| clean(d.get(k).and_then(|v| v.as_string()));
            Some(Raw {
                guid: get("Guid"),
                name: get("Name"),
                custom_command: get("Custom Command"),
                command: get("Command"),
                custom_directory: get("Custom Directory"),
                working_directory: get("Working Directory"),
            })
        })
        .collect()
}

pub fn from_dynamic_json(value: &serde_json::Value) -> Vec<Raw> {
    let Some(list) = value.get("Profiles").and_then(|v| v.as_array()) else { return Vec::new() };
    list.iter()
        .map(|p| {
            let get = |k: &str| clean(p.get(k).and_then(|v| v.as_str()));
            Raw {
                guid: get("Guid"),
                name: get("Name"),
                custom_command: get("Custom Command"),
                command: get("Command"),
                custom_directory: get("Custom Directory"),
                working_directory: get("Working Directory"),
            }
        })
        .collect()
}

/// `default_command` 是「没设自定义命令」时该跑什么(即内置默认 shell)。传进来
/// 而不是在这里算,纯粹是为了这个函数能在任何平台上测。
pub fn build(raws: &[Raw], default_command: &[String], home: Option<&Path>) -> Vec<Launcher> {
    let mut out = Vec::new();
    for raw in raws {
        let Some(name) = raw.name.clone() else { continue };

        let custom_cmd = matches!(raw.custom_command.as_deref(), Some("Yes") | Some("Custom Shell"));
        let command = if custom_cmd {
            match raw.command.as_deref().map(cmdline::split_posix) {
                Some(argv) if !argv.is_empty() => argv,
                // 标了自定义命令却没写命令:这条 profile 本身是坏的,跳过
                _ => continue,
            }
        } else {
            default_command.to_vec()
        };

        // "Recycle" 是「沿用上一个窗口的目录」,对我们没有意义(没有上一个窗口)
        let custom_dir = matches!(raw.custom_directory.as_deref(), Some("Yes") | Some("Advanced"));
        let cwd = if custom_dir {
            raw.working_directory.as_deref().map(|d| cmdline::expand_tilde(d, home))
        } else {
            None
        };

        // 命令和目录都跟默认一样 —— 这条 profile 的区别只在配色之类的地方,
        // 对"新建会话"来说它和内置默认完全等价,不进菜单
        if !custom_cmd && cwd.is_none() {
            continue;
        }

        let local_id = raw.guid.clone().unwrap_or_else(|| name.clone());
        out.push(Launcher::new(ID, &local_id, name, command).with_cwd(cwd));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_cmd() -> Vec<String> {
        vec!["/bin/zsh".to_string()]
    }

    fn raw(name: &str) -> Raw {
        Raw { name: Some(name.into()), guid: Some(format!("guid-{name}")), ..Default::default() }
    }

    #[test]
    fn custom_command_profiles_become_launchers() {
        let r = Raw { custom_command: Some("Yes".into()), command: Some("ssh prod01".into()), ..raw("Prod") };
        let out = build(&[r], &default_cmd(), None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "iterm2:guid-Prod");
        assert_eq!(out[0].name, "Prod");
        assert_eq!(out[0].command, ["ssh", "prod01"]);
    }

    #[test]
    fn custom_shell_is_treated_like_a_custom_command() {
        let r = Raw {
            custom_command: Some("Custom Shell".into()),
            command: Some("/opt/homebrew/bin/fish".into()),
            ..raw("Fish")
        };
        let out = build(&[r], &default_cmd(), None);
        assert_eq!(out[0].command, ["/opt/homebrew/bin/fish"]);
    }

    #[test]
    fn directory_only_profiles_use_the_default_shell() {
        let r = Raw {
            custom_directory: Some("Yes".into()),
            working_directory: Some("~/work".into()),
            ..raw("Work")
        };
        let home = Path::new("/Users/hiro");
        let out = build(&[r], &default_cmd(), Some(home));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].command, ["/bin/zsh"]);
        // 分隔符跟平台走,这里只关心 `~` 展开了(分隔符本身在 cmdline 里测)
        let want = home.join("work");
        assert_eq!(out[0].cwd.as_deref(), Some(&*want.to_string_lossy()));
    }

    #[test]
    fn appearance_only_profiles_are_skipped() {
        // 既没自定义命令也没自定义目录 —— 和内置默认等价
        let plain = Raw { custom_command: Some("No".into()), custom_directory: Some("No".into()), ..raw("Solarized") };
        let recycle = Raw { custom_directory: Some("Recycle".into()), ..raw("Recycle") };
        assert!(build(&[plain, recycle], &default_cmd(), None).is_empty());
    }

    #[test]
    fn broken_profiles_are_skipped_individually() {
        let no_name = Raw { name: None, custom_command: Some("Yes".into()), command: Some("sh".into()), ..Default::default() };
        let no_command = Raw { custom_command: Some("Yes".into()), command: None, ..raw("空") };
        let good = Raw { custom_command: Some("Yes".into()), command: Some("htop".into()), ..raw("好的") };
        let out = build(&[no_name, no_command, good], &default_cmd(), None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "好的");
    }

    #[test]
    fn reads_new_bookmarks_from_an_xml_plist() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0"><dict>
          <key>New Bookmarks</key>
          <array>
            <dict>
              <key>Guid</key><string>ABC-123</string>
              <key>Name</key><string>Prod SSH</string>
              <key>Custom Command</key><string>Yes</string>
              <key>Command</key><string>ssh prod01</string>
            </dict>
          </array>
        </dict></plist>"#;
        let value: plist::Value = plist::from_bytes(xml.as_bytes()).unwrap();
        let raws = from_prefs_plist(&value);
        assert_eq!(raws.len(), 1);
        assert_eq!(raws[0].guid.as_deref(), Some("ABC-123"));

        let out = build(&raws, &default_cmd(), None);
        assert_eq!(out[0].id, "iterm2:ABC-123");
        assert_eq!(out[0].command, ["ssh", "prod01"]);
    }

    #[test]
    fn reads_dynamic_profiles_from_json() {
        let json = serde_json::json!({
            "Profiles": [
                { "Name": "Dyn", "Guid": "D-1", "Custom Command": "Yes", "Command": "tmux attach -t main" }
            ]
        });
        let out = build(&from_dynamic_json(&json), &default_cmd(), None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "iterm2:D-1");
        assert_eq!(out[0].command, ["tmux", "attach", "-t", "main"]);
    }

    #[test]
    fn missing_keys_are_not_errors() {
        let value: plist::Value = plist::Value::Dictionary(Default::default());
        assert!(from_prefs_plist(&value).is_empty());
        assert!(from_dynamic_json(&serde_json::json!({})).is_empty());
    }
}
