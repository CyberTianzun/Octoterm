//! 开机自启。这一项不进 config.toml —— 它是 desktop 自己的行为,不是 server 的配置。
//!
//! macOS 用 LaunchAgent plist(放进 ~/Library/LaunchAgents 就会在下次登录时生效,
//! 不需要 launchctl),Windows 用 HKCU 的 Run 键。

use anyhow::{Context, Result};

pub const LABEL: &str = "com.octoterm.desktop";

#[cfg(any(target_os = "macos", windows))]
fn exe_path() -> Result<String> {
    Ok(std::env::current_exe()
        .context("无法取得自身路径")?
        .to_string_lossy()
        .into_owned())
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn plist_xml(exe: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
            label = LABEL,
            exe = xml_escape(exe),
        )
    }

    fn plist_path() -> Result<PathBuf> {
        let home = directories::BaseDirs::new().context("无法确定 HOME")?;
        Ok(home.home_dir().join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    pub fn is_enabled() -> Result<bool> {
        Ok(plist_path()?.exists())
    }

    pub fn set(enabled: bool) -> Result<()> {
        let path = plist_path()?;
        if enabled {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, plist_xml(&exe_path()?))
                .with_context(|| format!("无法写入 {}", path.display()))?;
        } else if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("无法删除 {}", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "octoterm";

    pub fn is_enabled() -> Result<bool> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ) else {
            return Ok(false);
        };
        Ok(run.get_value::<String, _>(VALUE).is_ok())
    }

    pub fn set(enabled: bool) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu
            .create_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)
            .context("无法打开 Run 注册表键")?;
        if enabled {
            // 加引号:路径里有空格时 Windows 才不会把它当成多个参数
            run.set_value(VALUE, &format!("\"{}\"", exe_path()?))
                .context("无法写入自启项")?;
        } else {
            match run.delete_value(VALUE) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("无法删除自启项"),
            }
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("octoterm-desktop 只支持 Windows 与 macOS");

#[cfg(any(target_os = "macos", windows))]
pub use imp::{is_enabled, set};

#[cfg(target_os = "macos")]
pub use imp::plist_xml;
