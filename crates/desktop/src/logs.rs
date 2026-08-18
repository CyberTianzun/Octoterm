//! GUI 进程没有可见的 stderr(macOS 是双击 .app,Windows 是 windows subsystem),
//! 所以日志必须落盘,托盘菜单用系统默认程序打开它。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 超过这个大小就在启动时清空。不做滚动归档 —— 一个随手用的工具不需要日志考古。
const MAX_LOG_BYTES: u64 = 1 << 20;

pub fn log_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "octoterm")
        .context("无法确定配置目录")?;
    Ok(dirs.config_dir().join("octoterm.log"))
}

pub fn truncate_if_larger_than(path: &Path, limit: u64) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > limit => std::fs::write(path, b""),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// 装上全局 subscriber,返回日志文件路径。
pub fn init() -> Result<PathBuf> {
    let path = log_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    truncate_if_larger_than(&path, MAX_LOG_BYTES)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("无法打开日志 {}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .init();
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_log_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.log");
        std::fs::write(&path, vec![b'x'; 4096]).unwrap();

        truncate_if_larger_than(&path, 1024).unwrap();

        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn a_small_log_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.log");
        std::fs::write(&path, b"hello").unwrap();

        truncate_if_larger_than(&path, 1024).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn a_missing_log_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        truncate_if_larger_than(&dir.path().join("nope.log"), 1024).unwrap();
    }
}
