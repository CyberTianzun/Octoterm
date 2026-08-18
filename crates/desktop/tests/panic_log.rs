//! panic 到底有没有落进日志文件 —— 端到端验证一次。
//!
//! 这条链子是 `logs.rs` 存在的理由的延伸:GUI 没有可见的 stderr,而 panic 消息只
//! 走 stderr;叠加 `panic = "abort"`,一次 wgpu / winit 的 panic 就是进程凭空消失、
//! 日志里一个字都没有。所以「hook 装上了、写了、而且 flush 了」必须钉死。
//!
//! 用子进程测:`std::panic::set_hook` 是进程全局的,在测试进程里装会和并行跑的
//! 其它测试互相踩(而且装完就摘不干净)。这里让测试二进制把自己重新拉起来一份,
//! 在那份里装 hook、真的 panic 一次,父进程回来读文件。

use std::path::Path;
use std::process::Command;

/// 有这个环境变量的就是被拉起来的那一份,它负责去 panic。
const CHILD: &str = "OCTOTERM_PANIC_LOG_CHILD";

const MESSAGE: &str = "烟雾测试:炸给日志看";

#[test]
fn a_panic_lands_in_the_log_file() {
    if let Ok(log) = std::env::var(CHILD) {
        octoterm_desktop::logs::install_panic_hook(Path::new(&log));
        panic!("{MESSAGE}");
    }

    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("octoterm.log");
    let exe = std::env::current_exe().expect("拿不到测试二进制自己的路径");
    let out = Command::new(exe)
        // `--exact` 是必须的:子进程跑的是同一个测试二进制,不限定就会把这个文件里
        // 的测试全跑一遍。
        .args(["a_panic_lands_in_the_log_file", "--exact"])
        .env(CHILD, &log)
        .output()
        .expect("拉不起子进程");
    assert!(!out.status.success(), "子进程 panic 了,不该是成功退出");

    let logged = std::fs::read_to_string(&log).expect("hook 根本没建出日志文件");
    assert!(logged.contains(MESSAGE), "panic 消息没进日志:{logged:?}");
    assert!(logged.contains("PANIC"), "缺少便于搜索的前缀:{logged:?}");
    assert!(
        logged.contains("panic_log.rs"),
        "缺少出事位置,用户拿着这份日志无从定位:{logged:?}"
    );
}
