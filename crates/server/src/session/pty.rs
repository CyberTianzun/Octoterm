use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use octoterm_protocol::SessionInfo;
use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

use super::buffer::SeqBuffer;
use super::grid::SessionGrid;

pub const BROADCAST_CAP: usize = 256;

#[derive(Debug, Clone)]
pub enum SessionOutput {
    Data { end_seq: u64, bytes: Bytes },
    Exited,
}

pub struct Snapshot {
    pub end_seq: u64,
    pub repaint: Vec<u8>,
}

struct Shared {
    name: String,
    grid: SessionGrid,
    buffer: SeqBuffer,
}

pub struct Session {
    pub id: u64,
    created_at: u64,
    shared: Mutex<Shared>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    tx: broadcast::Sender<SessionOutput>,
    exited: AtomicBool,
}

fn default_shell() -> Vec<String> {
    #[cfg(unix)]
    {
        vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())]
    }
    #[cfg(windows)]
    {
        // portable-pty 的 CreateProcessW 把 exe 放进 lpApplicationName,不会再搜 PATH。
        // 必须给绝对路径,否则 ConPTY 下经常 "系统找不到指定的文件"。
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let powershell =
            PathBuf::from(system_root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
        if powershell.is_file() {
            vec![powershell.to_string_lossy().into_owned(), "-NoLogo".into()]
        } else if let Ok(comspec) = std::env::var("ComSpec") {
            vec![comspec]
        } else {
            vec!["cmd.exe".into()]
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

impl Session {
    pub fn spawn(
        id: u64,
        name: String,
        cols: u16,
        rows: u16,
        command: Option<Vec<String>>,
        buffer_cap: usize,
    ) -> Result<Arc<Session>> {
        let argv = command.unwrap_or_else(default_shell);
        if argv.is_empty() || argv[0].is_empty() {
            bail!("empty command");
        }
        let cwd = default_cwd();
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
            shared: Mutex::new(Shared {
                name,
                grid: SessionGrid::new(cols, rows),
                buffer: SeqBuffer::new(buffer_cap),
            }),
            writer: Mutex::new(Some(writer)),
            master: Mutex::new(Some(pty.master)),
            killer: Mutex::new(killer),
            tx: tx.clone(),
            exited: AtomicBool::new(false),
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

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        {
            let guard = self.master.lock().unwrap();
            let master = guard.as_ref().context("session pty closed")?;
            master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        }
        self.shared.lock().unwrap().grid.resize(cols, rows);
        Ok(())
    }

    pub fn snapshot(&self) -> Snapshot {
        let shared = self.shared.lock().unwrap();
        Snapshot { end_seq: shared.buffer.end_seq(), repaint: shared.grid.repaint() }
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
    fn default_shell_program_is_usable() {
        let sh = default_shell();
        assert!(!sh.is_empty() && !sh[0].is_empty());
        #[cfg(windows)]
        {
            assert!(
                std::path::Path::new(&sh[0]).is_file(),
                "default shell must be an absolute existing path, got {:?}",
                sh[0]
            );
        }
    }

    #[test]
    fn empty_command_is_rejected() {
        let err = match Session::spawn(1, "t".into(), 80, 24, Some(vec![]), 64) {
            Ok(_) => panic!("empty command should fail"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("empty command"), "{err}");
    }
}
