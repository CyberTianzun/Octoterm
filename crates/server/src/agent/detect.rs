//! 「本机装没装这个 agent」。
//!
//! 规则是**三元证据**,不靠单一目录 —— 因为「`~/.claude` 存在」什么都证明不了:
//! 任何写过那个目录的程序都会创建它,octoterm 自己装 hook 就会。参考实现正是栽在
//! 这里,最后不得不把 claude-code 整个从默认检测里排除掉。
//!
//! 按可信度从高到低:配置文件里有**非我方**内容 > CLI 在 PATH 上 > 目录里有
//! 别的东西 > 没有。

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 检测所依赖的外部环境。**PATH 必须显式传入**,不在函数里读全局环境 ——
/// 否则单测会读到开发机上真实存在的 `claude`,把「没装」的用例全部染成「装了」。
pub struct DetectEnv {
    pub home: PathBuf,
    pub path: Option<OsString>,
}

impl DetectEnv {
    /// 真实进程环境。读不出 home 时回落到当前目录,让扫描退化成「什么都没找到」
    /// 而不是整个失败。
    pub fn current() -> Self {
        let home = directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self { home, path: std::env::var_os("PATH") }
    }
}

/// 在 PATH 上找一个可执行文件。Windows 上还要试 `.cmd` / `.exe` —— agent 的
/// CLI 基本都是 npm 装的,那边落地的是 `.cmd` shim 而不是裸名字。
pub fn on_path(env: &DetectEnv, exe: &str) -> Option<PathBuf> {
    let path = env.path.as_ref()?;
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{exe}.cmd"), format!("{exe}.exe"), format!("{exe}.bat"), exe.to_string()]
    } else {
        vec![exe.to_string()]
    };
    std::env::split_paths(path).find_map(|dir| {
        names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
    })
}

/// 目录里除了这些之外还有东西吗。用来把「只有我们写过的配置」和「用户真的用过」
/// 区分开。
pub fn dir_has_entries_besides(dir: &Path, ignore: &[&str]) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // `.DS_Store` 这类噪音不算证据
        !ignore.contains(&name.as_ref()) && !name.starts_with('.')
    })
}
