use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use bytes::Bytes;
use octoterm_protocol::SessionInfo;
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
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
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    tx: broadcast::Sender<SessionOutput>,
    exited: AtomicBool,
}

fn default_shell() -> Vec<String> {
    #[cfg(unix)]
    return vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())];
    #[cfg(windows)]
    return vec!["powershell.exe".into()];
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
        let pty = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        let argv = command.unwrap_or_else(default_shell);
        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        if let Ok(home) = std::env::var("HOME") {
            cmd.cwd(home);
        }
        // 客户端渲染器统一是 xterm 类(见 clients/web,基于 xterm.js),明确告知
        // shell/应用它能力所及,而不是继承宿主进程的 TERM(可能是别的终端类型
        // 或压根没设置),避免颜色/能力探测出错。
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        let mut child = pty.slave.spawn_command(cmd)?;
        drop(pty.slave);
        let killer = child.clone_killer();
        let mut reader = pty.master.try_clone_reader()?;
        let writer = pty.master.take_writer()?;
        let (tx, _) = broadcast::channel(BROADCAST_CAP);

        let session = Arc::new(Session {
            id,
            created_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            shared: Mutex::new(Shared {
                name,
                grid: SessionGrid::new(cols, rows),
                buffer: SeqBuffer::new(buffer_cap),
            }),
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
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
                    Ok(0) => break,
                    Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                    Ok(n) => {
                        let end_seq = {
                            let mut shared = s.shared.lock().unwrap();
                            shared.grid.advance(&buf[..n]);
                            shared.buffer.append(&buf[..n]);
                            shared.buffer.end_seq()
                        };
                        let _ = s.tx.send(SessionOutput::Data {
                            end_seq,
                            bytes: Bytes::copy_from_slice(&buf[..n]),
                        });
                    }
                }
            }
            let _ = child.wait();
            s.exited.store(true, Ordering::SeqCst);
            let _ = s.tx.send(SessionOutput::Exited);
        });

        Ok(session)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionOutput> {
        self.tx.subscribe()
    }

    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(bytes)?;
        w.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.lock().unwrap().resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
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
        let _ = self.killer.lock().unwrap().kill();
    }
}
