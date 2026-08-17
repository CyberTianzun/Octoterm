//! 内置 provider:系统默认的那个 shell。
//!
//! 它是唯一一个**保证非空**的 provider —— 别的来源都可能没装、没配、配坏了,
//! 这个必须永远给得出一条能用的命令,否则"新建会话"这个动作就没有底了。
//!
//! `command: null` 的 `new-session` 也走这里(见 [`default_command`]),这样
//! "菜单里的第一条"和"不选任何东西时的默认"天然是同一个东西,不会漂移。

use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

use super::{Launcher, LauncherProvider};

pub const ID: &str = "builtin";

pub struct Builtin;

impl LauncherProvider for Builtin {
    fn id(&self) -> &'static str {
        ID
    }
    fn discover(&self) -> anyhow::Result<Vec<Launcher>> {
        Ok(discover())
    }
}

/// 从路径里取一个像样的显示名:`/bin/zsh` → `zsh`。
fn stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

#[cfg(unix)]
pub fn discover() -> Vec<Launcher> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let name = stem(&shell);
    vec![Launcher::new(ID, "default", name, vec![shell])]
}

#[cfg(windows)]
pub fn discover() -> Vec<Launcher> {
    let mut out = Vec::new();
    // portable-pty 的 CreateProcessW 把 exe 放进 lpApplicationName,不会再搜 PATH。
    // 必须给绝对路径,否则 ConPTY 下经常 "系统找不到指定的文件"。
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let powershell =
        PathBuf::from(&system_root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    if powershell.is_file() {
        out.push(Launcher::new(
            ID,
            "powershell",
            "Windows PowerShell",
            vec![powershell.to_string_lossy().into_owned(), "-NoLogo".into()],
        ));
    }
    let comspec = std::env::var("ComSpec")
        .ok()
        .filter(|p| Path::new(p).is_file())
        .unwrap_or_else(|| PathBuf::from(&system_root).join(r"System32\cmd.exe").to_string_lossy().into_owned());
    out.push(Launcher::new(ID, "cmd", stem(&comspec), vec![comspec]));
    out
}

/// `new-session` 没指定命令时用哪个。
///
/// 定义成"菜单第一条",而不是另写一份默认值:两份真相迟早会对不上。
pub fn default_command() -> Vec<String> {
    if let Some(first) = discover().into_iter().next() {
        if !first.command.is_empty() {
            return first.command;
        }
    }
    // discover 理论上不会空,但默认 shell 是不能失败的东西,兜一层
    #[cfg(unix)]
    {
        vec!["/bin/sh".into()]
    }
    #[cfg(windows)]
    {
        vec!["cmd.exe".into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_is_never_empty_and_ids_are_prefixed() {
        let out = discover();
        assert!(!out.is_empty());
        for l in &out {
            assert!(l.id.starts_with("builtin:"), "{}", l.id);
            assert!(!l.command.is_empty());
            assert!(!l.name.is_empty());
        }
    }

    #[test]
    fn default_command_program_is_usable() {
        let argv = default_command();
        assert!(!argv.is_empty());
        let program = Path::new(&argv[0]);
        // Windows 上兜底可能是裸 `cmd.exe`(靠 PATH),unix 必须是存在的绝对路径
        if cfg!(unix) {
            assert!(
                program.is_absolute() && program.exists(),
                "默认 shell 必须是存在的绝对路径,实际是 {argv:?}"
            );
        }
    }

    #[test]
    fn default_command_matches_first_menu_entry() {
        assert_eq!(default_command(), discover()[0].command);
    }

    /// stem 走 `Path` 的语义,只认当前平台的分隔符 —— 这没问题,因为喂给它的
    /// 路径都来自当前平台自己的环境变量。
    #[test]
    fn stem_takes_the_program_name() {
        #[cfg(unix)]
        assert_eq!(stem("/bin/zsh"), "zsh");
        #[cfg(windows)]
        assert_eq!(stem(r"C:\Windows\System32\cmd.exe"), "cmd");
    }
}
