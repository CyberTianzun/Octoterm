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

/// 启动阶段的产物:配置本身,以及两条「起来了但没完全起来」的坏消息。
///
/// 这两个错误必须一路带到 GUI 里 —— 配置坏了 / 端口占用时 Web UI 是打不开的,
/// 设置窗口是用户唯一能看到原因、并且动手修的地方。
#[derive(Debug, Clone)]
pub struct Startup {
    pub config: Config,
    /// `Config::load` 失败的原因;此时 `config` 是默认值。
    pub config_error: Option<String>,
    /// 启动时 `Supervisor::restart` 失败的原因。保存成功重新监听后会被清掉。
    pub listen_error: Option<String>,
}

pub struct App {
    rt: Runtime,
    sup: Supervisor,
    tray: Option<Tray>,
    proxy: EventLoopProxy<UserEvent>,
    log_path: PathBuf,
    /// 启动阶段的产物:配置副本 + 启动失败原因。配置只做只读展示(window_size
    /// 与 [[launcher]] 设置界面不提供修改,改了也不会写回这个副本)。
    startup: Startup,
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
        startup: Startup,
    ) -> Self {
        Self {
            rt,
            sup,
            tray: None,
            proxy,
            log_path,
            startup,
            settings_window: None,
            view: None,
        }
    }

    /// 启动时的致命状况合成的一条常驻横幅。它和 [`crate::settings::ui::View::message`]
    /// 是两回事:横幅说的是「程序现在处于什么坏状态」,`message` 说的是「你刚点的
    /// 这次保存结果如何」——后者随编辑清除,前者一直挂到状况被修好为止。
    fn banner(&self) -> Option<String> {
        match (&self.startup.config_error, &self.startup.listen_error) {
            (Some(c), Some(l)) => Some(format!("配置文件有错:{c}\n监听失败:{l}")),
            (Some(c), None) => Some(format!("配置文件有错(当前使用默认值):{c}")),
            (None, Some(l)) => Some(format!("当前未监听:{l}")),
            (None, None) => None,
        }
    }

    /// 打开设置窗口时,从当前生效值构造视图。
    fn build_view(&self) -> crate::settings::ui::View {
        use crate::settings::{state::Form, ui::View};
        let listen = self.sup.listen().unwrap_or(self.startup.config.listen);
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
        let banner = self.banner();
        View {
            form,
            // WindowSize 只有 Smallest / Largest / Latest 三个单词,和 config.toml
            // 里写的小写形式一一对应。
            window_size: format!("{:?}", self.startup.config.window_size).to_lowercase(),
            launchers: self
                .startup
                .config
                .launchers
                .iter()
                .map(|l| (l.name.clone(), l.command.join(" ")))
                .collect(),
            banner,
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

        // 重新监听成功了,启动时那条「未监听」就此作废。不清掉的话托盘状态行和
        // 横幅会一直挂着一条已经不成立的错误(用户明明已经把端口改好了)。
        // config_error 不能跟着清:配置文件仍然是坏的 —— `configfile::save` 靠
        // toml_edit 解析原文,文件坏着的时候这次保存根本落不了盘。
        if applied.restarted {
            self.startup.listen_error = None;
        }

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

        let banner = self.banner();
        if let Some(view) = self.view.as_mut() {
            view.banner = banner;
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
            // 带上原因:托盘状态行是用户第一眼看到的东西,只说「未监听」等于
            // 让他去翻日志。
            None => match &self.startup.listen_error {
                Some(e) => format!("octoterm · 未监听({e})"),
                None => "octoterm · 未监听".to_string(),
            },
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

/// 退出前的清理:先停掉 HTTP 层(挡住新连接建新会话),再**显式**杀掉每一个
/// 已有的会话。
///
/// 不依赖 winit、不弹对话框,好让「退出会杀掉全部会话」这条能被测试直接验证。
///
/// ⚠️ 这个循环看着多余,其实一步都不能省 —— 靠 drop 是收不掉会话的:
/// `Session` 没有 `Drop` 实现,而 pty 的读线程和 wait 线程各自握着一份
/// `Arc<Session>`(见 `crates/server/src/session/pty.rs`),`Arc` 计数永远归不了
/// 零,`Session::kill()` / `killer.kill()` 因此从来不会被调用。现在进程退出后
/// 还能收拾干净,纯粹是靠 OS 关掉 pty master fd、内核给 slave 的前台进程组发
/// SIGHUP —— 被 `nohup` / `disown` 的孙进程、或者干脆忽略 SIGHUP 的程序会原地
/// 留成孤儿。只有走 `manager.kill(id)`(→ `Session::kill` → `killer.kill()` +
/// `force_close_pty()`)才是真的杀。**别把这个循环删了。**
pub fn shutdown(sup: &mut Supervisor) {
    // 先停 HTTP 层再杀会话,而不是反过来:kill 这几行期间如果 HTTP 层还活着,
    // 理论上会有一个 WebSocket 客户端在拿到 `manager.list()` 这份快照**之后**、
    // 循环把它 kill 掉**之前**发一条 `NewSession`——那个新会话就漏在快照外面,
    // 逃过这次 kill,变成孤儿。先 `stop()` 关掉 accept 循环能挡住*新连接*建
    // 新会话。
    //
    // 但这堵不死:`Supervisor::stop` 自己的文档写清楚了,已经升级完成的旧
    // WebSocket 不受 abort 影响,会带着旧 AppState 继续跑到客户端自己断开 ——
    // 这样的客户端理论上仍能在 `stop()` 之后、kill 循环跑到它之前发
    // `NewSession`。所以这个改动只是把竞争窗口从「HTTP 层整个存活期间」收窄到
    // 「一条已升级的旧 WebSocket 连接期间」,不是把这条竞争消除了。
    sup.stop();

    // 先把 id 收集出来再杀:`kill` 会改 manager 内部的 map,不能边遍历边改。
    let ids: Vec<u64> = sup.manager().list().iter().map(|s| s.id).collect();
    if !ids.is_empty() {
        tracing::info!(count = ids.len(), "退出:正在终止全部会话");
    }
    for id in ids {
        // 返回值(是否真的杀到了)故意丢弃:false 只代表会话在这期间自己退出了
        // (比如 shell 自然退出),是良性竞争。`SessionManager::kill` 在找不到
        // 会话时会自己打一条 `warn!("kill: no such session")`——这不是故障,
        // 退出日志里出现这条不代表哪里坏了,不必因此去改 server 侧的日志级别
        // (server 代码不允许改动)。
        sup.manager().kill(id);
    }
}

/// 「退出」按钮的文案。按钮上显示的文案与下面判断返回值时比较的字符串必须是
/// **同一份**:两处各写一遍字面量的话,以后改按钮文案很容易只改一处、忘了改
/// 比较那一处 —— macOS 上没有 `Ok` 兜底(见下方判断逻辑),那样会静默变成
/// 「有会话时永远退不出去」,而且不会有任何编译错误或测试失败提醒你。
const QUIT_LABEL: &str = "退出";
/// 同上,「取消」按钮的文案,和 `QUIT_LABEL` 共用同一处定义原因。
const CANCEL_LABEL: &str = "取消";

/// 有活跃会话时确认;没有会话就别打扰用户。返回 true 表示「确认退出」。
///
/// 用系统原生对话框而不是再开一个 egui 窗口:退出确认必须能在设置窗口没开的
/// 情况下弹出来,而且它是模态的 —— 借一个 winit 窗口反而要处理一堆生命周期。
///
/// 平台差异(rfd 0.15.4 实测,与 brief 的写法有出入):
/// * macOS 无父窗口时走 `CFUserNotificationDisplayAlert`,自定义按钮文案生效,
///   点第一个按钮返回 `Custom(QUIT_LABEL)`;
/// * Windows 在**没开** `common-controls-v6` 特性时,`OkCancelCustom` 会退化成
///   普通的 `MessageBoxW(MB_OKCANCEL)`,按钮是系统本地化的「确定 / 取消」,返回的
///   是 `MessageDialogResult::Ok` 而**不是** `Custom`。所以这里两种都认,只比
///   `Custom` 的话 Windows 上会永远退不出去。
///
/// (没有开 `common-controls-v6` 是有意的:那条路径走 `TaskDialogIndirect`,它只
/// 存在于 comctl32 v6,而要激活 v6 得给 exe 嵌一份 side-by-side 清单 —— 缺清单时
/// 是**加载期**符号解析失败,整个程序起不来,代价远大于按钮上少两个汉字。)
fn confirm_quit(sessions: usize) -> bool {
    let result = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("退出 octoterm")
        .set_description(format!(
            "还有 {sessions} 个会话正在运行。退出会终止它们,里面跑的程序都会被杀掉。"
        ))
        .set_buttons(rfd::MessageButtons::OkCancelCustom(QUIT_LABEL.into(), CANCEL_LABEL.into()))
        .show();
    matches!(result, rfd::MessageDialogResult::Custom(ref s) if s == QUIT_LABEL)
        || result == rfd::MessageDialogResult::Ok
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

            // 起不来的时候必须主动把窗口推到用户面前:这时候 Web UI 是打不开的,
            // 设置窗口是唯一的出路。
            //
            // 位置在 `self.tray.is_none()` 守卫**里面**:`resumed` 会被调用多次
            // (macOS 上每次 activate 都可能来一发),放外面就变成每次回到前台
            // 都弹一次设置窗口。
            if self.startup.config_error.is_some() || self.startup.listen_error.is_some() {
                self.proxy.send_event(UserEvent::MenuClicked(MenuAction::Settings)).ok();
            }
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
                // 「会话在断连后存活」正是这个项目的卖点,而内嵌模型下退出会把它们
                // 全杀掉 —— 这一下必须显眼。没有会话时不打扰用户。
                //
                // 对话框是同步模态的,弹着的时候事件循环整个卡住:这是原生模态的
                // 正常行为,退出确认本来也不该让用户还能去点别的菜单。
                let n = self.sup.manager().list().len();
                if n > 0 && !confirm_quit(n) {
                    return;
                }
                shutdown(&mut self.sup);
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
