//! 把 [`crate::agent::edit`] 的编辑计划安全地落到磁盘上。
//!
//! 这是整个 agent 集成里唯一有文件系统副作用的地方,顺序是硬的:
//!
//! ```text
//! 门控 → 读原文 → 解析(失败即中止) → 应用计划 → 备份原文 → tmp 写入 → rename
//! ```
//!
//! 备份在**解析成功之后、写入之前**:为一次注定失败的编辑留下备份只是垃圾。
//!
//! 与参考实现 clawd-on-desk 有意做的两处不同:
//!
//! 1. **备份落在 octoterm 自己的配置目录**,不是就地写 `.bak`。`~/.claude` 是别人的
//!    地方,我们不往里堆垃圾。
//! 2. **只在 install / uninstall 这种用户显式动作时写**,不做「每次 server 启动重写」。
//!    Claude Code 自己也会写这个文件(「Yes, and always allow」会往 `permissions` 里加
//!    规则),读-改-写的竞态窗口必须只出现在用户按下按钮的那一刻,而不是每次启动。

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::edit::{apply_to_json, ConfigEdit};

pub struct ApplyOpts {
    /// `agents.install_enabled`。默认关 —— 写别人的配置文件必须是显式选择。
    pub enabled: bool,
    pub backup_dir: PathBuf,
    pub backup_keep: usize,
}

#[derive(Debug)]
pub struct Outcome {
    pub path: PathBuf,
    /// 内容确有变化才为 true。无变化时既不写盘也不备份 —— 这让「装两次」在磁盘层
    /// 也是幂等的,而不只是在 JSON 层。
    pub changed: bool,
    pub backup: Option<PathBuf>,
}

#[derive(Debug)]
pub enum ApplyError {
    /// 开关关着。路由据此回 403,而不是 500。
    Disabled,
    /// 目标文件存在但不是合法 JSON。宁可整个失败也不覆盖 —— 那可能是用户手写坏的
    /// 配置,覆盖等于替他做了「丢掉」的决定。
    Invalid { path: PathBuf, detail: String },
    Io { path: PathBuf, detail: String },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "agent 集成的安装功能未启用(agents.install_enabled)"),
            Self::Invalid { path, detail } => {
                write!(f, "{} 不是合法 JSON,拒绝改写:{detail}", path.display())
            }
            Self::Io { path, detail } => write!(f, "读写 {} 失败:{detail}", path.display()),
        }
    }
}

impl std::error::Error for ApplyError {}

fn io_err(path: &Path, e: impl fmt::Display) -> ApplyError {
    ApplyError::Io { path: path.to_path_buf(), detail: e.to_string() }
}

pub fn apply(edits: &[ConfigEdit], opts: &ApplyOpts) -> Result<Vec<Outcome>, ApplyError> {
    if !opts.enabled {
        return Err(ApplyError::Disabled);
    }
    // 按文件分组:一个 agent 的一批编辑通常都落在同一个文件上,读一次、写一次
    let mut paths: Vec<&Path> = Vec::new();
    for e in edits {
        if !paths.contains(&e.path.as_path()) {
            paths.push(&e.path);
        }
    }
    paths.into_iter().map(|path| apply_one(path, edits, opts)).collect()
}

fn apply_one(path: &Path, edits: &[ConfigEdit], opts: &ApplyOpts) -> Result<Outcome, ApplyError> {
    let original = match std::fs::read_to_string(path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io_err(path, e)),
    };

    let mut doc: Value = match original.as_deref() {
        // 空文件当成空对象:Claude Code 自己也接受,而且这比报错更符合直觉
        Some(text) if text.trim().is_empty() => Value::Object(Default::default()),
        Some(text) => serde_json::from_str(text)
            .map_err(|e| ApplyError::Invalid { path: path.to_path_buf(), detail: e.to_string() })?,
        None => Value::Object(Default::default()),
    };

    for edit in edits.iter().filter(|e| e.path == path) {
        apply_to_json(&mut doc, &edit.op)
            .map_err(|e| ApplyError::Invalid { path: path.to_path_buf(), detail: e.to_string() })?;
    }

    let rendered = render(&doc);
    if original.as_deref() == Some(rendered.as_str()) {
        return Ok(Outcome { path: path.to_path_buf(), changed: false, backup: None });
    }

    let backup = match original.as_deref() {
        Some(text) => Some(save_backup(path, text, opts)?),
        None => None,
    };
    write_atomic(path, &rendered)?;
    Ok(Outcome { path: path.to_path_buf(), changed: true, backup })
}

/// 两空格缩进 + 末尾换行 —— Claude Code 自己写这个文件时就是这个形状,跟着它,
/// 免得每次它写一遍、我们写一遍就互相把对方的格式改掉。
fn render(doc: &Value) -> String {
    let mut s = serde_json::to_string_pretty(doc).unwrap_or_else(|_| "{}".into());
    s.push('\n');
    s
}

fn save_backup(path: &Path, text: &str, opts: &ApplyOpts) -> Result<PathBuf, ApplyError> {
    std::fs::create_dir_all(&opts.backup_dir).map_err(|e| io_err(&opts.backup_dir, e))?;
    let stem = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dest = opts.backup_dir.join(format!("{stem}.{ts}.bak"));
    std::fs::write(&dest, text).map_err(|e| io_err(&dest, e))?;
    prune_backups(&stem, opts);
    Ok(dest)
}

/// 只保留最近 `backup_keep` 份。修剪失败不是错误 —— 备份已经存下了,那才是要紧的。
fn prune_backups(stem: &str, opts: &ApplyOpts) {
    let Ok(entries) = std::fs::read_dir(&opts.backup_dir) else { return };
    let prefix = format!("{stem}.");
    let mut mine: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&prefix))
                .unwrap_or(false)
        })
        .collect();
    if mine.len() <= opts.backup_keep {
        return;
    }
    // 文件名里的时间戳是定宽递增的,字典序即时间序
    mine.sort();
    let drop = mine.len() - opts.backup_keep;
    for p in mine.into_iter().take(drop) {
        let _ = std::fs::remove_file(p);
    }
}

/// tmp + rename。tmp 必须和目标同目录,否则 rename 会跨文件系统而失败。
fn write_atomic(path: &Path, content: &str) -> Result<(), ApplyError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
    let tmp = dir.join(format!(
        ".{}.octoterm-{}.tmp",
        path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        std::process::id()
    ));
    std::fs::write(&tmp, content).map_err(|e| io_err(&tmp, e))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        // rename 失败就把中间文件收走,不给用户的目录留垃圾
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(path, e));
    }
    Ok(())
}

/// 备份放 octoterm 自己的配置目录下,不放 `~/.claude`。
pub fn default_backup_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "octoterm")
        .map(|d| d.config_dir().join("agent-backups"))
}
