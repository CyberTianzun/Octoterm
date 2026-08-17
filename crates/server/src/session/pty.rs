use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use octoterm_protocol::SessionInfo;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

use crate::config::WindowSize;

use super::buffer::SeqBuffer;
use super::grid::SessionGrid;

pub const BROADCAST_CAP: usize = 256;

/// 尺寸下界:某个客户端上报 1×1(手机上软键盘顶掉整个视口时真会发生)不该把
/// 会话压死,更不该让别人跟着一起被压死。
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 5;

#[derive(Debug, Clone)]
pub enum SessionOutput {
    Data { end_seq: u64, bytes: Bytes },
    /// 权威尺寸变了(见 `Session::apply_size`)。每个 attach 的泵都会把它转成
    /// `ServerMsg::Resized` 发给自己那一端。
    Resized { cols: u16, rows: u16 },
    Exited,
}

pub struct Snapshot {
    pub end_seq: u64,
    pub repaint: Vec<u8>,
    /// 这份重绘是按哪个尺寸渲染的——和 repaint 取自同一次加锁,不会错配。
    pub cols: u16,
    pub rows: u16,
}

/// 一个 attach 的尺寸诉求。`ord` 单调递增,Latest 策略据此挑出最近的那个。
struct Desired {
    cols: u16,
    rows: u16,
    ord: u64,
}

struct Shared {
    name: String,
    grid: SessionGrid,
    buffer: SeqBuffer,
    viewports: HashMap<u64, Desired>,
    next_ord: u64,
}

pub struct Session {
    pub id: u64,
    created_at: u64,
    window_size: WindowSize,
    shared: Mutex<Shared>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    tx: broadcast::Sender<SessionOutput>,
    exited: AtomicBool,
    next_viewport: AtomicU64,
}

/// 按策略把所有 attach 的尺寸诉求归并成 pty 的权威尺寸。
///
/// 没有任何 attach 时返回 `None`,调用方保持当前尺寸不变:最后一个客户端离开
/// 时把会话弹回默认 80×24,只会让里面的应用白白重排一次。
fn effective_size(policy: WindowSize, viewports: &HashMap<u64, Desired>) -> Option<(u16, u16)> {
    let (cols, rows) = match policy {
        WindowSize::Smallest => (
            viewports.values().map(|v| v.cols).min()?,
            viewports.values().map(|v| v.rows).min()?,
        ),
        WindowSize::Largest => (
            viewports.values().map(|v| v.cols).max()?,
            viewports.values().map(|v| v.rows).max()?,
        ),
        WindowSize::Latest => {
            let latest = viewports.values().max_by_key(|v| v.ord)?;
            (latest.cols, latest.rows)
        }
    };
    Some((cols.max(MIN_COLS), rows.max(MIN_ROWS)))
}

/// 一个 attach 在会话尺寸表里的席位。析构即摘除并重算 —— detach、连接断开、
/// 泵被 abort 全走这一条路,不会把已经走掉的客户端留在表里锁着别人的尺寸。
pub struct Viewport {
    session: Arc<Session>,
    id: u64,
}

impl Viewport {
    /// 更新本 attach 的尺寸诉求。是否真的落到 pty 上由策略决定(G2)。
    pub fn set(&self, cols: u16, rows: u16) -> Result<()> {
        self.session.set_viewport(self.id, cols, rows)
    }
}

impl Drop for Viewport {
    fn drop(&mut self) {
        self.session.shared.lock().unwrap().viewports.remove(&self.id);
        if let Err(e) = self.session.apply_size() {
            tracing::debug!(session = self.session.id, error = %e, "resize after detach failed");
        }
    }
}

/// 选一个真实存在的启动目录。Windows 上 `$HOME` 经常没设,或是 Git Bash
/// 的 `/c/Users/...`,CreateProcess/ConPTY 都不能用。
fn default_cwd() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(windows)]
    {
        if let Ok(p) = std::env::var("USERPROFILE") {
            candidates.push(PathBuf::from(p));
        }
        if let (Ok(drive), Ok(path)) = (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
            candidates.push(PathBuf::from(format!("{drive}{path}")));
        }
    }
    if let Ok(p) = std::env::var("HOME") {
        candidates.push(PathBuf::from(p));
    }
    if let Some(dirs) = directories::UserDirs::new() {
        candidates.push(dirs.home_dir().to_path_buf());
    }
    for c in candidates {
        if c.is_dir() {
            return Some(c);
        }
    }
    std::env::current_dir().ok().filter(|p| p.is_dir())
}

/// 「跑什么、在哪跑」。这两个总是一起来的 —— 它们出自同一条 launcher(见
/// `crate::launcher`),分开传只会让调用点多两个可以搞混的 `None`。
#[derive(Debug, Clone, Default)]
pub struct Launch {
    /// `None` = 用内置默认 shell
    pub command: Option<Vec<String>>,
    /// `None` 或指向不存在的目录 = 用服务端默认启动目录
    pub cwd: Option<String>,
}

impl Session {
    pub fn spawn(
        id: u64,
        name: String,
        cols: u16,
        rows: u16,
        launch: Launch,
        buffer_cap: usize,
        window_size: WindowSize,
    ) -> Result<Arc<Session>> {
        let mut argv = launch.command.unwrap_or_else(crate::launcher::builtin::default_command);
        if argv.is_empty() || argv[0].is_empty() {
            bail!("empty command");
        }
        // portable-pty 把 argv[0] 塞进 CreateProcessW 的 lpApplicationName,
        // 不会像 Windows Terminal 那样试探带空格的未加引号路径。客户端也可能
        // 把已经拆坏的 argv 发回来,spawn 这一层再粘一次当最后防线。
        #[cfg(windows)]
        {
            argv = crate::launcher::cmdline::glue_unquoted_windows_exe(argv, &|p| {
                std::path::Path::new(p).is_file()
            });
        }
        // 请求的目录不存在就回落到默认,而不是让整个 spawn 失败:profile 里的
        // 目录可能是在另一台机器上写的,为此拒绝开会话是过度反应。
        let cwd = launch
            .cwd
            .map(PathBuf::from)
            .filter(|p| {
                let ok = p.is_dir();
                if !ok {
                    tracing::warn!(session = id, cwd = %p.display(), "请求的启动目录不存在,回落到默认");
                }
                ok
            })
            .or_else(default_cwd);
        tracing::info!(session = id, ?argv, cwd = ?cwd, "spawning session");

        let pty = native_pty_system()
            .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .with_context(|| format!("openpty failed for session {id}"))?;

        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        if let Some(ref cwd) = cwd {
            cmd.cwd(cwd);
            // Git-for-Windows 常把 HOME 设成 /c/Users/...,子进程继承后会把工作目录搞乱
            #[cfg(windows)]
            {
                let home_ok = std::env::var_os("HOME").is_some_and(|h| PathBuf::from(h).is_dir());
                if !home_ok {
                    cmd.env("HOME", cwd);
                }
            }
        }
        // 客户端渲染器统一是 xterm 类(见 clients/web,基于 xterm.js),明确告知
        // shell/应用它能力所及,而不是继承宿主进程的 TERM(可能是别的终端类型
        // 或压根没设置),避免颜色/能力探测出错。
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        let mut child = pty
            .slave
            .spawn_command(cmd)
            .with_context(|| format!("spawn {argv:?} in cwd {cwd:?} failed"))?;
        drop(pty.slave);
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader().context("clone pty reader")?;
        let writer = pty.master.take_writer().context("take pty writer")?;
        let (tx, _) = broadcast::channel(BROADCAST_CAP);

        let session = Arc::new(Session {
            id,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            window_size,
            shared: Mutex::new(Shared {
                name,
                grid: SessionGrid::new(cols, rows),
                buffer: SeqBuffer::new(buffer_cap),
                viewports: HashMap::new(),
                next_ord: 0,
            }),
            writer: Mutex::new(Some(writer)),
            master: Mutex::new(Some(pty.master)),
            killer: Mutex::new(killer),
            tx: tx.clone(),
            exited: AtomicBool::new(false),
            next_viewport: AtomicU64::new(1),
        });

        // 阻塞读线程:pty 输出 → grid + 环形缓冲 + 广播
        let s = session.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        tracing::debug!(session = s.id, "pty read eof");
                        break;
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        tracing::warn!(session = s.id, error = %e, "pty read ended");
                        break;
                    }
                    Ok(n) => {
                        let (end_seq, replies) = {
                            let mut shared = s.shared.lock().unwrap();
                            shared.grid.advance(&buf[..n]);
                            shared.buffer.append(&buf[..n]);
                            let replies = shared.grid.take_pty_replies();
                            (shared.buffer.end_seq(), replies)
                        };
                        // 必须在释放 grid 锁之后回写,避免和 write_input 锁顺序缠死
                        if !replies.is_empty() {
                            if let Err(e) = s.write_input(&replies) {
                                tracing::warn!(
                                    session = s.id,
                                    error = %e,
                                    "failed to write terminal query reply"
                                );
                            } else {
                                tracing::debug!(
                                    session = s.id,
                                    bytes = replies.len(),
                                    "wrote terminal query reply"
                                );
                            }
                        }
                        let _ = s.tx.send(SessionOutput::Data {
                            end_seq,
                            bytes: Bytes::copy_from_slice(&buf[..n]),
                        });
                    }
                }
            }
            s.mark_exited();
        });

        // Windows ConPTY:子进程退出后读端经常不 EOF。必须另线程 wait,再关掉
        // PTY 才能让读线程结束、会话从 manager 里摘掉。
        let s = session.clone();
        std::thread::spawn(move || {
            match Child::wait(&mut *child) {
                Ok(status) => {
                    tracing::info!(
                        session = s.id,
                        exit_code = status.exit_code(),
                        "child process exited"
                    );
                }
                Err(e) => tracing::warn!(session = s.id, error = %e, "wait on child failed"),
            }
            s.force_close_pty();
            s.mark_exited();
        });

        Ok(session)
    }

    fn mark_exited(&self) {
        if self.exited.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = self.tx.send(SessionOutput::Exited);
    }

    fn force_close_pty(&self) {
        if let Some(w) = self.writer.lock().unwrap().take() {
            drop(w);
        }
        if let Some(m) = self.master.lock().unwrap().take() {
            drop(m);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionOutput> {
        self.tx.subscribe()
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        let mut guard = self.writer.lock().unwrap();
        let w = guard.as_mut().context("session pty closed")?;
        w.write_all(bytes).context("write pty input")?;
        w.flush().context("flush pty input")?;
        Ok(())
    }

    /// 登记一个 attach 的尺寸诉求,返回它在尺寸表里的席位(析构即摘除)。
    pub fn viewport(self: &Arc<Self>, cols: u16, rows: u16) -> Viewport {
        let id = self.next_viewport.fetch_add(1, Ordering::SeqCst);
        let vp = Viewport { session: self.clone(), id };
        // pty 已经关掉的会话仍然允许 attach:客户端马上会收到 session-exited。
        let _ = vp.set(cols, rows);
        vp
    }

    fn set_viewport(&self, id: u64, cols: u16, rows: u16) -> Result<()> {
        {
            let mut shared = self.shared.lock().unwrap();
            let ord = shared.next_ord;
            shared.next_ord += 1;
            shared.viewports.insert(id, Desired { cols, rows, ord });
        }
        self.apply_size()
    }

    /// 重算权威尺寸并落到 pty + grid。尺寸没变就什么都不做——否则每个客户端的
    /// 每一次 refit 都会给里面的应用来一发 SIGWINCH。变了就广播,所有 attach
    /// 都会收到 `resized`,包括触发这次变化的那一端。
    fn apply_size(&self) -> Result<()> {
        let mut shared = self.shared.lock().unwrap();
        let Some((cols, rows)) = effective_size(self.window_size, &shared.viewports) else {
            return Ok(()); // 没有 attach:保持当前尺寸
        };
        if shared.grid.size() == (cols, rows) {
            return Ok(());
        }
        {
            let guard = self.master.lock().unwrap();
            let master = guard.as_ref().context("session pty closed")?;
            master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        }
        shared.grid.resize(cols, rows);
        drop(shared);
        let _ = self.tx.send(SessionOutput::Resized { cols, rows });
        Ok(())
    }

    pub fn size(&self) -> (u16, u16) {
        self.shared.lock().unwrap().grid.size()
    }

    pub fn snapshot(&self) -> Snapshot {
        let shared = self.shared.lock().unwrap();
        let (cols, rows) = shared.grid.size();
        Snapshot { end_seq: shared.buffer.end_seq(), repaint: shared.grid.repaint(), cols, rows }
    }

    pub fn replay_from(&self, seq: u64) -> Option<(u64, Vec<u8>)> {
        let shared = self.shared.lock().unwrap();
        shared.buffer.read_from(seq).map(|b| (shared.buffer.end_seq(), b))
    }

    pub fn info(&self) -> SessionInfo {
        let shared = self.shared.lock().unwrap();
        let (cols, rows) = shared.grid.size();
        SessionInfo {
            id: self.id,
            name: shared.name.clone(),
            cols,
            rows,
            created_at: self.created_at,
        }
    }

    pub fn rename(&self, name: &str) {
        self.shared.lock().unwrap().name = name.to_string();
    }

    pub fn kill(&self) {
        // portable-pty 0.9 的 WinChildKiller 把 TerminateProcess 成功/失败弄反了,
        // 返回值不可信;真正拆会话靠关 PTY + wait 线程。
        if let Err(e) = self.killer.lock().unwrap().kill() {
            tracing::debug!(session = self.id, error = %e, "child kill returned error");
        }
        tracing::info!(session = self.id, "kill requested");
        self.force_close_pty();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cwd_is_existing_dir() {
        let cwd = default_cwd().expect("should resolve a cwd");
        assert!(cwd.is_dir(), "{cwd:?} is not a directory");
        #[cfg(windows)]
        {
            let s = cwd.to_string_lossy();
            assert!(
                !s.starts_with('/') || s.chars().nth(1) == Some(':'),
                "Windows cwd must be a Win32 path, got {s}"
            );
        }
    }

    #[test]
    fn empty_command_is_rejected() {
        let launch = Launch { command: Some(vec![]), cwd: None };
        let spawn = Session::spawn(1, "t".into(), 80, 24, launch, 64, WindowSize::default());
        let err = match spawn {
            Ok(_) => panic!("empty command should fail"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("empty command"), "{err}");
    }

    /// Git for Windows 写进 WT 的 commandline 不带引号,切分后是
    /// `["C:\Program", "Files\Git\bin\bash.exe", "-li"]`。spawn 必须把它粘回去,
    /// 否则 CreateProcessW 去找一个叫 `C:\Program` 的文件。
    #[cfg(windows)]
    #[test]
    fn spawn_glues_unquoted_git_bash_path() {
        let bash = r"C:\Program Files\Git\bin\bash.exe";
        if !std::path::Path::new(bash).is_file() {
            return;
        }
        let launch = Launch {
            command: Some(vec![
                r"C:\Program".into(),
                r"Files\Git\bin\bash.exe".into(),
                "-lc".into(),
                "exit 0".into(),
            ]),
            cwd: None,
        };
        let session = Session::spawn(1, "t".into(), 80, 24, launch, 64, WindowSize::default())
            .expect("unquoted Git Bash path must be glued before CreateProcessW");
        session.kill();
    }

    /// 不存在的 cwd 不该让 spawn 失败 —— profile 里的目录可能来自另一台机器。
    #[test]
    fn nonexistent_cwd_falls_back_instead_of_failing() {
        let launch =
            Launch { command: None, cwd: Some("/definitely/not/a/real/dir/for/octoterm".into()) };
        let session = Session::spawn(1, "t".into(), 80, 24, launch, 64, WindowSize::default())
            .expect("spawn should survive a bad cwd");
        session.kill();
    }

    /// 按 attach 顺序建表:ord 就是数组下标,Latest 取最后一个。
    fn viewports(sizes: &[(u16, u16)]) -> HashMap<u64, Desired> {
        sizes
            .iter()
            .enumerate()
            .map(|(i, &(cols, rows))| (i as u64, Desired { cols, rows, ord: i as u64 }))
            .collect()
    }

    #[test]
    fn no_viewport_keeps_current_size() {
        assert_eq!(effective_size(WindowSize::Smallest, &viewports(&[])), None);
    }

    #[test]
    fn smallest_takes_the_min_of_each_dimension_independently() {
        // 宽的那个矮、窄的那个高:两个维度分别取最小,谁都不会被截断。
        let vps = viewports(&[(120, 24), (80, 40)]);
        assert_eq!(effective_size(WindowSize::Smallest, &vps), Some((80, 24)));
        assert_eq!(effective_size(WindowSize::Largest, &vps), Some((120, 40)));
        assert_eq!(effective_size(WindowSize::Latest, &vps), Some((80, 40)));
    }

    #[test]
    fn extreme_report_is_clamped_to_the_floor() {
        let vps = viewports(&[(120, 40), (1, 1)]);
        assert_eq!(effective_size(WindowSize::Smallest, &vps), Some((MIN_COLS, MIN_ROWS)));
    }
}
