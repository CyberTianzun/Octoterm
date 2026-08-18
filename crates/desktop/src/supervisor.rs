//! 内嵌 server 的生命周期。
//!
//! 全部要点只有一句:`SessionManager` 由 Supervisor **长期持有**,跨 restart 不
//! 重建。HTTP 层(listener + AppState)可以随便拆了重搭,pty 会话一个都不会丢 ——
//! 但这只对**新连接**成立:新连接会连到新的 HTTP 层;已经升级完成的旧
//! WebSocket 不会被这次 restart 打断,会继续跑在旧的 AppState 上,直到客户端
//! 自己断开。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use octoterm_server::app::{serve, AppState};
use octoterm_server::config::{LauncherSpec, WindowSize};
use octoterm_server::launcher::LauncherProvider;
use octoterm_server::session::manager::SessionManager;
use tokio::net::TcpListener;

/// 端口刚被自己 abort 释放时,重试几次(见 `bind_with_retry`)。
const BIND_RETRIES: usize = 10;
const BIND_RETRY_INTERVAL: Duration = Duration::from_millis(20);

struct Running {
    listen: SocketAddr,
    token: String,
    join: tokio::task::JoinHandle<()>,
}

pub struct Supervisor {
    manager: Arc<SessionManager>,
    launchers: Arc<Vec<Box<dyn LauncherProvider>>>,
    running: Option<Running>,
}

impl Supervisor {
    pub fn new(buffer_cap: usize, window_size: WindowSize, specs: &[LauncherSpec]) -> Self {
        Self {
            manager: SessionManager::new(buffer_cap, window_size),
            launchers: Arc::new(octoterm_server::launcher::providers(specs)),
            running: None,
        }
    }

    pub fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }

    pub fn listen(&self) -> Option<SocketAddr> {
        self.running.as_ref().map(|r| r.listen)
    }

    pub fn token(&self) -> Option<&str> {
        self.running.as_ref().map(|r| r.token.as_str())
    }

    /// 用新的监听地址与 token 重建 HTTP 层。返回实际监听到的地址(端口 0 会被
    /// 系统换成真实端口)。
    ///
    /// 地址**有变化**时先 bind 新的、成功了才关旧的 —— 端口被占用这种最常见的
    /// 失败不会把用户锁在外面。地址**没变**时做不到先 bind(同一地址上不能有两个
    /// listener,`SO_REUSEPORT` 在 Windows 上不可用,`SO_REUSEADDR` 在 Windows
    /// 上的语义是抢占),只能先关再带重试地 bind。
    ///
    /// 注意这两条路径失败后的状态不同:地址有变化时失败了旧的还在跑,`self` 原封
    /// 不动;地址没变时旧的已经关掉了,重试仍失败就只能停在"没有 HTTP 层"上,
    /// 由调用方决定是提示用户还是换个地址再来。
    ///
    /// **空 token 一律拒绝**,见函数体开头的 `ensure!`。
    pub async fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr> {
        // 这里是所有 token 进入 `AppState` 的**唯一收口**(desktop 里只有 main.rs 的
        // 启动路径和 app.rs 的 `AppEffects` 两个调用方,都得过这一关)。server 侧的
        // `bearer_ok` 与 WebSocket 握手都是 `token == state.token` 的直接比较,空
        // token 对空 token 一律放行 —— 等于把鉴权整个关掉;配上用户改成 `0.0.0.0`
        // 的监听地址,就是一个全网卡、无鉴权的终端服务。
        //
        // 两个刻意的选择:
        // 1. 用 `ensure!` 而不是 `debug_assert!` —— release 构建里断言是不存在的,
        //    而这是一条安全边界,必须在正式构建里也拦得住;
        // 2. 放在函数**最前面**,在任何 bind / stop 之前 —— 同地址那条路径会先
        //    `stop()`,晚一步拦就会变成「已经把旧的关了才发现 token 是空的」。
        //
        // 顺带把纯空白也算作空:HTTP 头里的 `Authorization: Bearer   ` 经过解析后
        // 剩不下什么,当 token 用和空串没有区别。
        ensure!(
            !token.trim().is_empty(),
            "拒绝用空 token 启动 HTTP 层:server 侧鉴权会全部放行"
        );
        let same_addr = self.running.as_ref().is_some_and(|r| r.listen == listen);
        let (listener, actual) = if same_addr {
            self.stop();
            let l = bind_with_retry(listen).await?;
            let actual = l
                .local_addr()
                .with_context(|| format!("无法读取 {listen} 上的实际监听地址"))?;
            (l, actual)
        } else {
            let l = TcpListener::bind(listen)
                .await
                .with_context(|| format!("无法监听 {listen}"))?;
            // local_addr 也可能失败,所以要赶在 stop() 之前读完:等旧的关掉了才
            // 发现读不出新地址,就会留下「旧的已关、新的没记账」的空档。
            let actual = l
                .local_addr()
                .with_context(|| format!("无法读取 {listen} 上的实际监听地址"))?;
            self.stop();
            (l, actual)
        };
        let state = AppState {
            manager: self.manager.clone(),
            token: token.clone(),
            launchers: self.launchers.clone(),
        };
        let join = tokio::spawn(async move {
            if let Err(e) = serve(listener, state).await {
                tracing::error!(error = %format!("{e:#}"), "http 层异常退出");
            }
        });
        tracing::info!(%actual, "http 层已就绪");
        self.running = Some(Running { listen: actual, token, join });
        Ok(actual)
    }

    /// 停掉 HTTP 层。会话与 `SessionManager` 不受影响。
    ///
    /// 用 abort 而不是 axum 的 graceful shutdown:graceful 会等所有连接结束,而
    /// WebSocket 是长连接,永远不结束 —— 那等于挂死。
    ///
    /// 已知限制:abort 只终止 accept 循环并释放监听端口,**已经升级完成的
    /// WebSocket 会继续用旧的 AppState(旧 token)跑下去**。原因是 axum 的 ws
    /// 回调跑在 `on_upgrade` 自己 `tokio::spawn` 出去的独立任务里,持有的是 accept
    /// 出来的独立 fd,abort 掉 serve 任务够不着它。范围仅限于此:普通的 HTTP
    /// keep-alive 连接反而会被收掉 —— serve 的 future 被 drop 时 `signal_rx` 一并
    /// 没了,每条连接任务上的 `signal_tx.closed()` 会触发
    /// `conn.graceful_shutdown()`。
    ///
    /// 落到用户身上:不管 restart 是否换了监听地址,已经升级完成的旧 WebSocket
    /// 都不受影响 —— 会带着旧 token 一直活到客户端自己断开;真正变化的只是
    /// **新连接**连去哪(换地址后连新地址,同地址轮换 token 后新连接才用得上
    /// 新 token)。换句话说,restart 从来都不是甩掉一条旧连接或旧 token 的有效
    /// 手段。要做到立即失效得由 server 侧支持,这里够不着。
    pub fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.join.abort();
            tracing::info!(listen = %r.listen, "http 层已停止");
        }
    }
}

impl Drop for Supervisor {
    /// 别把 accept 循环连同它占着的端口一起漏掉,也别把会话漏了。
    ///
    /// 目前只有托盘「退出」这一条路会调用 `app::shutdown()` 显式逐个
    /// `manager.kill(id)`。`App` 自己没有 `Drop`,如果 `run_app` 因为别的原因
    /// (非 `Quit` 菜单项触发的提前返回)结束,`App` 被丢弃时孤儿会话问题会原样
    /// 回来 —— 这里补一道保底:`Supervisor` 被丢弃时,把当时还挂着的会话也
    /// 一并杀掉。
    ///
    /// 和 `app::shutdown()` 里的显式循环不冲突:走过 `shutdown()` 的正常退出
    /// 路径此时 `manager.list()` 已经是空的,这里的循环是空转。
    ///
    /// 不会拖慢或卡住进程退出:`Session::kill()`(→ `killer.kill()` +
    /// `force_close_pty()`)只是发出终止信号、关掉 pty 的 writer/master 句柄,
    /// 不等待子进程被 wait 线程真正 reap,是同步且非阻塞的。abort 同样不要求
    /// 当前在 runtime 里,runtime 已经关掉时也是安全的空操作。
    fn drop(&mut self) {
        let ids: Vec<u64> = self.manager.list().iter().map(|s| s.id).collect();
        for id in ids {
            // 返回值丢弃的原因见 `app::shutdown()` 里同样的调用点:false 只是
            // 良性竞争(会话已经自己退出),不代表这里出了问题。
            self.manager.kill(id);
        }
        self.stop();
    }
}

/// 端口刚被自己 abort 释放,操作系统那边是异步的,给几次机会。
///
/// `JoinHandle::abort` 只是打个标记,任务要等 runtime 下一次调度到它才会被丢弃、
/// listener 才真正关闭 —— 所以第一次 bind 几乎必然撞 `EADDRINUSE`,循环里的
/// sleep 同时也是让出执行权给那个待丢弃的任务。
async fn bind_with_retry(listen: SocketAddr) -> Result<TcpListener> {
    let mut last = None;
    for attempt in 0..BIND_RETRIES {
        match TcpListener::bind(listen).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                last = Some(e);
                // 最后一次失败之后没有下一轮了,不必再白等一个 interval。
                if attempt + 1 < BIND_RETRIES {
                    tokio::time::sleep(BIND_RETRY_INTERVAL).await;
                }
            }
        }
    }
    Err(last.expect("循环至少跑一次")).with_context(|| format!("无法监听 {listen}"))
}
