//! GUI 进程没有可见的 stderr(macOS 是双击 .app,Windows 是 windows subsystem),
//! 所以日志必须落盘,托盘菜单用系统默认程序打开它。

use std::any::Any;
use std::io::Write;
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
        std::fs::create_dir_all(dir)
            .with_context(|| format!("无法创建目录 {}", dir.display()))?;
    }
    truncate_if_larger_than(&path, MAX_LOG_BYTES)
        .with_context(|| format!("无法清空日志 {}", path.display()))?;
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
    install_panic_hook(&path);
    Ok(path)
}

/// 从 panic 的载荷里取出人能读的那段消息。
///
/// `panic!("字面量")` 装的是 `&'static str`,`panic!("{}")` 之类(以及 `unwrap`)
/// 装的是 `String`,两种都要认。再有别的类型就没什么可说了,给一句占位而不是把
/// 整条 panic 记录丢掉。
fn payload_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s
    } else {
        "(无法识别的 panic 载荷)"
    }
}

/// 把一次 panic 拼成写进日志的那一行。
///
/// 纯函数,从 hook 里拆出来就是为了能直接测各种载荷和缺位置的情形(整条链子的
/// 端到端验证在 `tests/panic_log.rs`,那个得开子进程)。
///
/// 前缀是大写的 `PANIC`,和 tracing 那些 `INFO` / `ERROR` 行一眼能区分开:用户把
/// 日志发过来时,搜这一个词就能定位。
fn format_panic(message: &str, location: Option<String>) -> String {
    let at = location.unwrap_or_else(|| "位置未知".to_string());
    // 消息本身可能是多行的(assert_eq! 的输出就是),压成一行 —— 否则在日志里看
    // 起来像好几条互不相干的记录。
    let message = message.replace('\n', " / ");
    format!("PANIC 于 {at}:{message}")
}

/// 装上 panic hook,把 panic 的消息和位置写进日志文件。
///
/// 这个模块存在的全部理由就是「GUI 没有可见的 stderr」,而 panic 消息偏偏只走
/// stderr;叠加根 `Cargo.toml` 的 `panic = "abort"`,一次 wgpu / egui / winit 的
/// panic 就是:进程瞬间消失、所有 pty 会话被内核收掉、日志里一个字都没有,用户
/// 完全无从诊断。
///
/// 有意直接写文件而不是走 `tracing::error!`:panic 完全可能发生在某个日志宏正
/// 持有 subscriber 那把写锁的时候,hook 里再进 tracing 就是自己等自己。hook 里
/// 也不做任何可能 panic 的事(全是 `let _ =`,没有 unwrap)—— hook 里再 panic
/// 会直接变成 abort,连这一行都留不下。
///
/// 正常路径上由 [`init`] 调用。`pub` 是为了 `tests/panic_log.rs` 能在一个**子进程**
/// 里端到端验证它:`set_hook` 是进程全局的,同进程里测会和别的测试互相踩。
pub fn install_panic_hook(path: &Path) {
    // 保留默认 hook 而不是整个替换:从终端启动时 stderr 上那一份、以及
    // `RUST_BACKTRACE=1` 的调用栈都还有用,打包成 .app 之后它只是没人看得见,
    // 并不碍事。先写自己这一行再交给它,免得默认 hook 出岔子把我们的记录吞掉。
    let previous = std::panic::take_hook();
    let path = path.to_path_buf();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let line = format_panic(payload_message(info.payload()), location);
        let opened = std::fs::OpenOptions::new().create(true).append(true).open(&path);
        if let Ok(mut file) = opened {
            let _ = writeln!(file, "{line}");
            // `panic = "abort"` 下 hook 一返回进程立刻就没了,没有任何析构或缓冲
            // 刷新的机会 —— 必须在这里就把字节推到磁盘上。
            let _ = file.flush();
            let _ = file.sync_data();
        }
        previous(info);
    }));
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

    // 这里只测两个纯函数;hook 真的装上、真的写盘那条链子在 `tests/panic_log.rs`
    // 里跑(`set_hook` 是进程全局的,同进程里装会和并行跑的其它测试互相踩)。

    #[test]
    fn a_str_payload_is_read() {
        // `panic!("字面量")` 的形态
        assert_eq!(payload_message(&"炸了"), "炸了");
    }

    #[test]
    fn a_string_payload_is_read() {
        // `panic!("{}", x)`、`unwrap()` 的形态
        assert_eq!(payload_message(&"炸了".to_string()), "炸了");
    }

    #[test]
    fn an_unknown_payload_still_yields_something() {
        assert!(!payload_message(&42u8).is_empty(), "认不出载荷也不能丢掉整条记录");
    }

    #[test]
    fn a_panic_line_carries_message_and_location() {
        let line = format_panic("炸了", Some("src/window.rs:12:5".to_string()));
        assert_eq!(line, "PANIC 于 src/window.rs:12:5:炸了");
    }

    #[test]
    fn a_panic_without_location_is_still_logged() {
        let line = format_panic("炸了", None);
        assert!(line.contains("炸了"), "位置未知不该把消息也丢了:{line}");
    }

    #[test]
    fn a_multiline_panic_is_flattened_to_one_line() {
        let line = format_panic("左边:1\n右边:2", Some("a.rs:1:1".to_string()));
        assert!(!line.contains('\n'), "多行 panic 在日志里会被看成几条记录:{line}");
        assert!(line.contains("左边:1") && line.contains("右边:2"), "内容不能丢:{line}");
    }
}
