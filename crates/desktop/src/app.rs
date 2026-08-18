//! winit 的 ApplicationHandler:所有状态都在主线程上,tokio 在后台线程。

use std::net::IpAddr;
use std::sync::Arc;

use octoterm_server::session::manager::SessionManager;
use tokio::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::WindowId;

use crate::supervisor::Supervisor;
use crate::tray::Tray;
use crate::window::EguiWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    OpenWeb,
    CopyUrl,
    Settings,
    ViewLogs,
    Quit,
}

#[derive(Debug, Clone)]
pub enum UserEvent {
    MenuClicked(MenuAction),
    SessionsChanged,
}

/// 把监听地址的 IP 部分整理成用户能直接用的样子:`0.0.0.0` / `::` 这类「监听所有
/// 网卡」的地址,人是没法拿它当访问地址敲进浏览器的,换成 `127.0.0.1`;IPv6 地址
/// 拼进 URL 需要方括号包起来。`status_text()` 和 `url()` 共用这一份,保证状态行
/// 显示的 host 和「复制访问链接」给出的 host 永远一致。
fn display_host(ip: IpAddr) -> String {
    if ip.is_unspecified() {
        "127.0.0.1".to_string()
    } else if ip.is_ipv6() {
        format!("[{ip}]")
    } else {
        ip.to_string()
    }
}

pub struct App {
    rt: Runtime,
    sup: Supervisor,
    tray: Option<Tray>,
    proxy: EventLoopProxy<UserEvent>,
    log_path: std::path::PathBuf,
    /// 平时是 None——托盘常驻应用不该在后台挂着一个隐藏窗口和一整套 GPU 上下文。
    /// 点「设置…」时创建,关窗口时置回 None,`EguiWindow` 的 Drop 负责把 surface
    /// 和 device 还给系统。
    settings_window: Option<EguiWindow>,
}

impl App {
    pub fn new(
        rt: Runtime,
        sup: Supervisor,
        proxy: EventLoopProxy<UserEvent>,
        log_path: std::path::PathBuf,
    ) -> Self {
        Self { rt, sup, tray: None, proxy, log_path, settings_window: None }
    }

    /// 带 token 的访问 URL,和 CLI 启动时打印的那一行是同一格式。
    fn url(&self) -> Option<String> {
        let listen = self.sup.listen()?;
        let token = self.sup.token()?;
        Some(format!("http://{}:{}/#token={token}", display_host(listen.ip()), listen.port()))
    }

    /// 状态行里能实际打开的 host。和 `url()` 用同一个 `display_host`,不然配置成
    /// `0.0.0.0` 时状态行显示 `0.0.0.0:7683`,用户照着敲进浏览器却打不开——
    /// 「复制访问链接」给的却是 `127.0.0.1`,两处得说一样的话。
    fn status_text(&self) -> String {
        match self.sup.listen() {
            Some(addr) => {
                let n = self.sup.manager().list().len();
                format!("octoterm · {}:{} · {n} 个会话", display_host(addr.ip()), addr.port())
            }
            None => "octoterm · 未监听".to_string(),
        }
    }

    fn refresh_status(&mut self) {
        let text = self.status_text();
        if let Some(tray) = self.tray.as_mut() {
            tray.set_status(&text);
        }
    }

    /// 订阅会话事件,变化时叫主线程刷新状态行。
    fn watch_sessions(&self, manager: Arc<SessionManager>) {
        let proxy = self.proxy.clone();
        self.rt.spawn(async move {
            let mut rx = manager.events();
            // Lagged 只说明事件来得太快、漏了几条,而状态行本来就只是重新读一次
            // 当前列表 —— 漏掉的那几条不影响结果,继续听就是;只有 Closed 才收工。
            while let Ok(_) | Err(RecvError::Lagged(_)) = rx.recv().await {
                if proxy.send_event(UserEvent::SessionsChanged).is_err() {
                    break; // 事件循环没了,收工
                }
            }
        });
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // 托盘常驻应用启动时不建窗口:设置窗口在用户点「设置…」时才创建。
        if self.tray.is_none() {
            match Tray::new(self.proxy.clone()) {
                Ok(tray) => {
                    tracing::info!("托盘已就绪");
                    self.tray = Some(tray);
                }
                Err(e) => {
                    tracing::error!(error = %e, "托盘创建失败");
                    return;
                }
            }
            self.watch_sessions(self.sup.manager().clone());
            self.refresh_status();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::SessionsChanged => self.refresh_status(),
            UserEvent::MenuClicked(MenuAction::OpenWeb) => match self.url() {
                Some(url) => {
                    if let Err(e) = open::that_detached(url) {
                        tracing::error!(error = %e, "打开浏览器失败");
                    }
                }
                // http 层没监听成功(比如端口被占用),点了菜单却什么都不会发生——
                // 不留日志的话用户根本无从判断是没反应还是自己没点中。
                None => tracing::warn!("尚未监听,无法打开 Web 客户端"),
            },
            UserEvent::MenuClicked(MenuAction::CopyUrl) => match self.url() {
                Some(url) => match arboard::Clipboard::new().and_then(|mut c| c.set_text(url)) {
                    Ok(()) => {}
                    Err(e) => tracing::error!(error = %e, "复制到剪贴板失败"),
                },
                None => tracing::warn!("尚未监听,无法复制访问链接"),
            },
            UserEvent::MenuClicked(MenuAction::ViewLogs) => {
                if let Err(e) = open::that_detached(&self.log_path) {
                    tracing::error!(error = %e, "打开日志失败");
                }
            }
            UserEvent::MenuClicked(MenuAction::Settings) => {
                if self.settings_window.is_none() {
                    match EguiWindow::open(event_loop, "octoterm 设置", (460, 420)) {
                        Ok(w) => self.settings_window = Some(w),
                        Err(e) => tracing::error!(error = %e, "无法打开设置窗口"),
                    }
                }
                // 窗口已经开着的话,这一下就当「把它叫到前面来」。
                if let Some(w) = &self.settings_window {
                    w.focus();
                    w.request_redraw();
                }
            }
            UserEvent::MenuClicked(MenuAction::Quit) => {
                self.sup.stop();
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(w) = self.settings_window.as_mut().filter(|w| w.id() == id) else {
            return;
        };
        // 先喂给 egui:它要靠这些事件维护自己的输入状态(按键、光标、缩放因子),
        // 返回值说明它有没有把这个事件吃掉。这里我们只关心窗口生命周期,不用管。
        let _consumed = w.on_window_event(&event);
        match event {
            WindowEvent::CloseRequested => {
                // 关窗口只是关窗口,程序继续常驻。置 None 触发 Drop,GPU 资源当场
                // 还回去——不是 set_visible(false) 那种「藏起来留着」。
                self.settings_window = None;
            }
            WindowEvent::RedrawRequested => {
                w.redraw_ui(|ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.label("设置界面将在下一步实现");
                    });
                });
            }
            _ => w.request_redraw(),
        }
    }
}
