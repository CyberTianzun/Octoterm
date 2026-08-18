//! 进程锁。两份 desktop 同时跑必然抢同一个端口,直接在启动时挡掉。
//!
//! 用文件锁而不是端口探测:端口可能被别的程序占着,那不代表已有 desktop 实例。
//! 操作系统在进程死亡时自动释放文件锁,所以崩溃不会留下僵尸锁。

use std::fs::{File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result};
use fs4::{FileExt, TryLockError};

/// 持有它就代表持有锁;drop 即释放。
pub struct Guard(File);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// `Ok(None)` 表示已经有另一个实例在跑(不是错误,是正常分支)。
pub fn acquire(path: &Path) -> Result<Option<Guard>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("无法创建目录 {}", dir.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("无法打开锁文件 {}", path.display()))?;
    match FileExt::try_lock(&file) {
        Ok(()) => Ok(Some(Guard(file))),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(e).context("锁文件失败"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_acquire_on_the_same_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.lock");

        let first = acquire(&path).unwrap();
        assert!(first.is_some(), "第一个实例应当拿到锁");

        let second = acquire(&path).unwrap();
        assert!(second.is_none(), "第二个实例应当被拒绝");
    }

    #[test]
    fn releasing_the_guard_lets_the_next_one_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.lock");

        drop(acquire(&path).unwrap());
        assert!(acquire(&path).unwrap().is_some(), "锁没被释放");
    }
}
