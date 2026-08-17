//! 用户自己写的 launcher,来自 octoterm 的 config.toml:
//!
//! ```toml
//! [[launcher]]
//! name = "prod ssh"
//! command = ["ssh", "prod01"]
//! cwd = "~/work"          # 可选
//! ```
//!
//! 这是"扩展"的第一档:不需要装插件、不需要改代码,而且因为 `command` 直接就是
//! argv,不用去猜任何一套命令行切分规则。系统终端的 profile 表达不了的东西
//! (或者用户压根不用 iTerm2 / Windows Terminal)都落在这里。

use super::{cmdline, Launcher, LauncherProvider};
use crate::config::LauncherSpec;

pub const ID: &str = "config";

pub struct UserConfig {
    specs: Vec<LauncherSpec>,
}

impl UserConfig {
    pub fn new(specs: Vec<LauncherSpec>) -> Self {
        Self { specs }
    }
}

impl LauncherProvider for UserConfig {
    fn id(&self) -> &'static str {
        ID
    }

    fn discover(&self) -> anyhow::Result<Vec<Launcher>> {
        let home = directories::UserDirs::new().map(|d| d.home_dir().to_path_buf());
        Ok(build(&self.specs, home.as_deref()))
    }
}

/// 纯函数形式,方便测。`home` 只用于展开 `cwd` 里的 `~`。
pub fn build(specs: &[LauncherSpec], home: Option<&std::path::Path>) -> Vec<Launcher> {
    let mut out = Vec::new();
    for spec in specs {
        // 一条写坏的配置只丢它自己:用户手上可能有十条能用的,不该被第十一条拖累
        if spec.command.is_empty() || spec.command[0].trim().is_empty() {
            tracing::warn!(name = %spec.name, "config launcher 的 command 为空,已跳过");
            continue;
        }
        let name = if spec.name.trim().is_empty() { spec.command[0].clone() } else { spec.name.clone() };
        // id 用名字而不是下标:在 config.toml 里挪动顺序不该让"上次用的那个"失忆
        out.push(
            Launcher::new(ID, &name, name.clone(), spec.command.clone())
                .with_cwd(spec.cwd.as_deref().map(|c| cmdline::expand_tilde(c, home))),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn spec(name: &str, command: &[&str], cwd: Option<&str>) -> LauncherSpec {
        LauncherSpec {
            name: name.into(),
            command: command.iter().map(|s| s.to_string()).collect(),
            cwd: cwd.map(|s| s.into()),
        }
    }

    #[test]
    fn builds_launchers_with_tilde_expanded_cwd() {
        let out = build(&[spec("prod ssh", &["ssh", "prod01"], Some("~/work"))], Some(Path::new("/home/h")));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "config:prod ssh");
        assert_eq!(out[0].name, "prod ssh");
        assert_eq!(out[0].command, ["ssh", "prod01"]);
        assert_eq!(out[0].cwd.as_deref(), Some("/home/h/work"));
        assert_eq!(out[0].detail, "ssh prod01");
    }

    #[test]
    fn empty_command_is_skipped_not_fatal() {
        let out = build(
            &[spec("坏的", &[], None), spec("好的", &["sh"], None), spec("也坏", &[" "], None)],
            None,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "好的");
    }

    #[test]
    fn blank_name_falls_back_to_the_program() {
        let out = build(&[spec("  ", &["htop"], None)], None);
        assert_eq!(out[0].name, "htop");
        assert_eq!(out[0].id, "config:htop");
    }
}
