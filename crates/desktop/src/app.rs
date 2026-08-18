//! winit 的 ApplicationHandler:所有状态都在主线程上,tokio 在后台线程。

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use octoterm_server::config::Config;
use octoterm_server::session::manager::SessionManager;
use tokio::runtime::Runtime;
use tokio::sync::broadcast::error::RecvError;
use winit::application::ApplicationHandler;
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
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
    log_path: PathBuf,
    /// 启动时读到的配置。这里只用它只读展示 window_size 与 [[launcher]] ——
    /// 这两项设置界面不提供修改,改了也不会写回这个副本。
    config: Config,
    /// 平时是 None——托盘常驻应用不该在后台挂着一个隐藏窗口和一整套 GPU 上下文。
    /// 点「设置…」时创建,关窗口时置回 None,`EguiWindow` 的 Drop 负责把 surface
    /// 和 device 还给系统。
    settings_window: Option<EguiWindow>,
    /// 设置窗口的界面状态,和 `settings_window` 同生共死(开窗时建、关窗时清)。
    view: Option<crate::settings::ui::View>,
}

impl App {
    pub fn new(
        rt: Runtime,
        sup: Supervisor,
        proxy: EventLoopProxy<UserEvent>,
        log_path: PathBuf,
        config: Config,
    ) -> Self {
        Self {
            rt,
            sup,
            tray: None,
            proxy,
            log_path,
            config,
            settings_window: None,
            view: None,
        }
    }

    /// 打开设置窗口时,从当前生效值构造视图。
    fn build_view(&self) -> crate::settings::ui::View {
        use crate::settings::{state::Form, ui::View};
        let listen = self.sup.listen().unwrap_or(self.config.listen);
        // 读不出来就当没开:这一项读失败不该阻止用户打开设置窗口
        let autostart = crate::autostart::is_enabled().unwrap_or(false);
        let mut form = Form::from_current(listen, autostart);
        // token 回填当前生效值:用户多半是来看它、复制它的,展示为空反而费解。
        // 没在监听时压根没有「当前生效的 token」,框里就是空的(= 不固定)——
        // 保存时 save.rs 会现场生成一个,不会拿空串去跑。
        form.token = match self.sup.token() {
            Some(t) => t.to_string(),
            None => String::new(),
        };
        View {
            form,
            // WindowSize 只有 Smallest / Largest / Latest 三个单词,和 config.toml
            // 里写的小写形式一一对应。
            window_size: format!("{:?}", self.config.window_size).to_lowercase(),
            launchers: self
                .config
                .launchers
                .iter()
                .map(|l| (l.name.clone(), l.command.join(" ")))
                .collect(),
            message: None,
        }
    }

    /// 执行「保存并应用」。真正的分支逻辑在 [`crate::settings::save::apply`],
    /// 这里只负责把当前状态喂进去、把结果贴回界面。
    fn apply_settings(&mut self) {
        use crate::settings::save::{apply, Current};

        let Some(view) = self.view.as_ref() else { return };
        // 先把表单拷出来:下面要 &mut 借 self.sup,不能同时挂着 self.view 的借用。
        let form = view.form.clone();
        let current = Current {
            // 没在监听时是 None 而不是空串:空串会一路被当成一个真 token 用,
            // 而 server 侧 `token == state.token` 的比较对空串一律放行。
            listen: self.sup.listen(),
            token: self.sup.token().map(str::to_string),
            sessions: self.sup.manager().list().len(),
        };

        let mut fx = AppEffects { rt: &self.rt, sup: &mut self.sup };
        let applied = apply(&form, &current, &mut fx);

        // 无条件刷新,不看 `applied.restarted`:restart 失败时它是 false,而
        // 「同地址只换 token」那条失败路径恰恰会让服务停在未监听状态(supervisor
        // 已经先 stop() 了)——只在成功时刷新的话,状态行会一直挂着已经失效的
        // 旧地址和会话数。刷新本身只是重读一次当前状态,多调用没有代价。
        self.refresh_status();

        // 把**实际生效**的 token 写回表单。「表单留空 + 服务没在监听」时 save.rs 会
        // 当场生成一个新 token,不写回的话用户在这个窗口里根本看不到它是什么 ——
        // 退路只有关窗去托盘点「复制访问链接」,而界面上没有任何提示告诉他这一点。
        //
        // 只在 restarted 时写:校验失败、自启失败这些路径上服务没动过,写回去只会
        // 把用户正在输入的内容冲掉。写进去之后再点一次保存会把它固化进 config.toml,
        // 这和开窗时 `build_view` 回填当前 token 的效果一致,不是新增的行为。
        //
        // 位置:`RedrawRequested` 里 `redraw_ui` 闭包的**外面**(闭包同一帧可能被
        // 跑两趟),`apply_settings` 整个都在闭包外,这里同样是一次性副作用。
        if applied.restarted
            && let Some(token) = self.sup.token().map(str::to_string)
            && let Some(view) = self.view.as_mut()
        {
            view.form.token = token;
        }

        if let Some(view) = self.view.as_mut() {
            view.message = Some(applied.message);
        }
    }

    /// 关掉设置窗口:GPU 资源随 `EguiWindow` 的 Drop 当场还回去,界面状态一并
    /// 丢掉——下次开窗要重新读当前生效值,而不是接着上次没保存的输入往下改。
    fn close_settings(&mut self) {
        self.settings_window = None;
        self.view = None;
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

/// [`crate::settings::save::Effects`] 的真实实现:保存流程要做的 IO 都在这儿。
///
/// 单独一个结构体而不是给 `App` 实现:`apply` 要 `&mut` 借 supervisor,而同一时刻
/// 界面状态(`App::view`)还得留着写回提示文案,借整个 `App` 就冲突了。
struct AppEffects<'a> {
    rt: &'a Runtime,
    sup: &'a mut Supervisor,
}

impl crate::settings::save::Effects for AppEffects<'_> {
    fn set_autostart(&mut self, enabled: bool) -> Result<()> {
        crate::autostart::set(enabled)
    }

    fn config_path(&mut self) -> Result<PathBuf> {
        crate::configfile::default_path()
    }

    fn save_config(&mut self, path: &Path, edit: &crate::configfile::Editable) -> Result<()> {
        crate::configfile::save(path, edit)
    }

    fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr> {
        // 有意用 block_on 把 UI 线程卡住:换来的是保存流程完全串行、界面上不存在
        // 「正在保存」这种中间态。一次点击卡这一下(bind + spawn,毫秒级)可以接受。
        self.rt.block_on(self.sup.restart(listen, token))
    }

    fn new_token(&mut self) -> String {
        // 和 server 的 resolve_token 用同一种格式(32 位无连字符十六进制)
        uuid::Uuid::new_v4().simple().to_string()
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
                    tracing::error!(error = %format!("{e:#}"), "托盘创建失败");
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
                        tracing::error!(error = %format!("{e:#}"), "打开浏览器失败");
                    }
                }
                // http 层没监听成功(比如端口被占用),点了菜单却什么都不会发生——
                // 不留日志的话用户根本无从判断是没反应还是自己没点中。
                None => tracing::warn!("尚未监听,无法打开 Web 客户端"),
            },
            UserEvent::MenuClicked(MenuAction::CopyUrl) => match self.url() {
                Some(url) => match arboard::Clipboard::new().and_then(|mut c| c.set_text(url)) {
                    Ok(()) => {}
                    Err(e) => tracing::error!(error = %format!("{e:#}"), "复制到剪贴板失败"),
                },
                None => tracing::warn!("尚未监听,无法复制访问链接"),
            },
            UserEvent::MenuClicked(MenuAction::ViewLogs) => {
                if let Err(e) = open::that_detached(&self.log_path) {
                    tracing::error!(error = %format!("{e:#}"), "打开日志失败");
                }
            }
            UserEvent::MenuClicked(MenuAction::Settings) => {
                if self.settings_window.is_none() {
                    // 每次开窗都重新读一遍当前生效值,而不是复用上次关窗时的表单:
                    // 期间监听地址可能已经被别处改过了。
                    self.view = Some(self.build_view());
                    match EguiWindow::open(event_loop, "octoterm 设置", (460, 460)) {
                        Ok(w) => self.settings_window = Some(w),
                        Err(e) => {
                            tracing::error!(error = %format!("{e:#}"), "无法打开设置窗口");
                            self.view = None;
                        }
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
                self.close_settings();
            }
            WindowEvent::RedrawRequested => {
                use crate::settings::ui::{draw, Outcome};
                // 这个闭包同一帧里可能被调用多次(Grid 之类的容器会让 egui 重跑一
                // 趟),所以只能描述界面、只能往 outcome 上赋值,一次性的副作用
                // 全部挪到闭包外做一次。
                let mut outcome = Outcome::None;
                if let Some(view) = self.view.as_mut() {
                    w.redraw_ui(|ui| outcome = draw(ui, view));
                }
                let closed = w.close_requested();
                match outcome {
                    Outcome::None => {}
                    Outcome::Save => self.apply_settings(),
                    Outcome::Cancel => self.close_settings(),
                    Outcome::OpenConfigFile => match crate::configfile::default_path() {
                        Ok(p) => {
                            if let Err(e) = open::that_detached(&p) {
                                tracing::error!(error = %format!("{e:#}"), "打开配置文件失败");
                            }
                        }
                        Err(e) => tracing::error!(error = %format!("{e:#}"), "无法确定配置文件位置"),
                    },
                    Outcome::RegenerateToken => {
                        if let Some(view) = self.view.as_mut() {
                            // 和 server 的 resolve_token 用同一种格式(32 位十六进制)
                            view.form.token = uuid::Uuid::new_v4().simple().to_string();
                        }
                    }
                }
                // 界面里 `send_viewport_cmd(ViewportCommand::Close)` 的结果从这里
                // 出来:窗口自己关不掉自己,得由持有它的这一层置 None。设置界面
                // 目前不发这条命令(「取消」走的是 `Outcome::Cancel`),但
                // `EguiWindow` 的约定要求每帧看一眼,漏了以后加就会静默失效。
                if closed {
                    self.close_settings();
                }
                // 保存结果、重新生成的 token 这些都是刚刚才写进 view 的,而这一帧
                // 已经画完了 —— 不主动再要一帧,用户点完按钮会看不到任何变化。
                if outcome != Outcome::None
                    && let Some(w) = &self.settings_window
                {
                    w.request_redraw();
                }
            }
            _ => w.request_redraw(),
        }
    }

    /// `WaitUntil` 到点只是把事件循环叫醒,它自己不产生重绘请求——光标闪烁、
    /// tooltip 延迟显示这类靠非零 `repaint_delay` 排期的东西,得在这里补一脚。
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. })
            && let Some(w) = &self.settings_window
        {
            w.request_redraw();
        }
    }

    /// 每轮事件处理完、真正去睡之前决定「睡多久」。
    ///
    /// 没有窗口时是 `Wait`(无限期阻塞,直到有事件)——托盘常驻应用空闲时必须真的
    /// 空闲,不能靠 `Poll` 忙转。有窗口且 egui 排了定时重绘时才 `WaitUntil`,到点
    /// 由上面的 `new_events` 把重绘请求发出去。
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let flow = match self.settings_window.as_ref().and_then(|w| w.repaint_at()) {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        };
        event_loop.set_control_flow(flow);
    }
}
