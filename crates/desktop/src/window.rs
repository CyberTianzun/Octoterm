//! 设置窗口的载体:一个按需创建、关掉就真的销毁的 egui 窗口。
//!
//! 不用 eframe:它假定自己拥有事件循环、并在启动时就建窗口,而托盘常驻应用要的
//! 恰恰是「平时 0 窗口」。多出来的这点管线代码换的是空闲时进程里只剩 tokio 和一
//! 个状态栏图标。
//!
//! 渲染后端是 wgpu 而不是 glow:OpenGL 在 macOS 自 10.14 起已废弃,wgpu 走
//! Metal / DX12,是受支持的原生路径。
//!
//! 和事件循环的约定(调用方 `app.rs` 必须照做,否则光标不闪、`send_viewport_cmd`
//! 关不掉窗口):
//!
//! 1. 收到 `RedrawRequested` 时调 [`EguiWindow::redraw_ui`];
//! 2. 画完看 [`EguiWindow::close_requested`],为真就把窗口置 `None`(界面里
//!    `ctx.send_viewport_cmd(ViewportCommand::Close)` 的结果会从这里出来);
//! 3. `about_to_wait` 里按 [`EguiWindow::repaint_at`] 设 `ControlFlow`:`Some(t)`
//!    → `WaitUntil(t)`,`None` → `Wait`(没窗口时也是 `Wait`,托盘常驻应用空闲时
//!    必须真的空闲);
//! 4. `StartCause::ResumeTimeReached` 时调 [`EguiWindow::request_redraw`] ——
//!    `WaitUntil` 到点只是把事件循环叫醒,它自己不会产生重绘请求。

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

/// 只有一个窗口,所以整个进程共用 `ViewportId::ROOT` 这一个视口。
const VIEWPORT: egui::ViewportId = egui::ViewportId::ROOT;

pub struct EguiWindow {
    // 字段顺序即析构顺序,别调:`painter` 里的 `wgpu::Surface` 自己持有一份
    // `Arc<Window>`,得先让 painter 落地把 surface 还回去,`window` 最后一个 drop
    // 时引用计数才会真的归零、系统窗口才会真的消失。
    painter: egui_wgpu::winit::Painter,
    egui_state: egui_winit::State,
    egui_ctx: egui::Context,
    /// 攒着还没上传成功的纹理增量,见 [`Self::redraw_ui`] 里的说明。
    textures_delta: egui::epaint::textures::TexturesDelta,
    /// 视口的当前状态(标题 / 位置 / 焦点 / 是否被要求关闭)。既要喂给 egui 当输入,
    /// 也是 `process_viewport_commands` 的输出落点。
    viewport_info: egui::ViewportInfo,
    /// 下一次该主动重绘的时刻;`None` = 没有待办的定时重绘。
    repaint_at: Option<Instant>,
    /// 界面通过 `ViewportCommand::Close` 要求关窗。
    close_requested: bool,
    window: Arc<Window>,
}

impl EguiWindow {
    pub fn open(event_loop: &ActiveEventLoop, title: &str, size: (u32, u32)) -> Result<Self> {
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1))
            .with_resizable(true);
        let window = Arc::new(event_loop.create_window(attrs).context("无法创建窗口")?);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            VIEWPORT,
            &window,
            Some(window.scale_factor() as f32),
            event_loop.system_theme(),
            None,
        );

        let mut viewport_info = egui::ViewportInfo::default();
        // `is_init = true`:只有第一次允许在 macOS 上查询最大化/最小化状态,之后再查
        // 会死锁(egui#3494)。
        egui_winit::update_viewport_info(&mut viewport_info, &egui_ctx, &window, true);

        // `Painter::new` 只建 wgpu::Instance,真正选设备要等 `set_window`——它得看
        // 到窗口才知道该挑哪块能画到这个 surface 上的适配器。两步都是 async,但这
        // 里是主线程的事件循环回调,不能 .await,用 pollster 就地转成阻塞调用。
        let mut painter = pollster::block_on(egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            egui_wgpu::WgpuConfiguration::default(),
            // 不要透明背板:要了在部分平台上会强制走 alpha 合成,而设置窗口是不透
            // 明的普通窗口。
            false,
            egui_wgpu::RendererOptions::default(),
        ));
        pollster::block_on(painter.set_window(VIEWPORT, Some(window.clone())))
            .context("无法初始化 wgpu 渲染器")?;

        Ok(Self {
            painter,
            egui_state,
            egui_ctx,
            textures_delta: Default::default(),
            viewport_info,
            repaint_at: None,
            close_requested: false,
            window,
        })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 界面是否通过 `ctx.send_viewport_cmd(ViewportCommand::Close)` 要求关窗。
    ///
    /// 窗口自己关不掉自己(`EguiWindow` 的所有权在 `App` 手里),所以调用方每次
    /// [`Self::redraw_ui`] 之后都要看一眼这个标志。
    pub fn close_requested(&self) -> bool {
        self.close_requested
    }

    /// 下一次该主动重绘的时刻,给事件循环拿去设 `ControlFlow::WaitUntil`。
    ///
    /// `None` 有两种含义,但对调用方是同一件事(设 `ControlFlow::Wait` 就行):
    /// 要么 egui 说「不用再画了」,要么它要的是「立刻再画一帧」——后者已经在
    /// [`Self::redraw_ui`] 里直接 `request_redraw()` 了,`Wait` 不会拦住已经挂起
    /// 的重绘请求。
    pub fn repaint_at(&self) -> Option<Instant> {
        self.repaint_at
    }

    /// 窗口已经开着的时候再点一次「设置…」,把它叫到前面来。
    ///
    /// macOS 的 `ActivationPolicy::Accessory` 下光 `makeKeyAndOrderFront:` 是不够的
    /// ——进程本身不在前台,窗口也上不来。winit 0.30 的 `focus_window()` 内部已经
    /// 先调了 `NSApplication::activateIgnoringOtherApps(true)` 再
    /// `makeKeyAndOrderFront:`,所以这条不用我们自己补;但它整段被
    /// `if !is_minimized && is_visible` 包着,窗口被最小化时会直接什么都不做,
    /// 所以这里先把它从 Dock 里捞回来。
    pub fn focus(&self) {
        self.window.set_minimized(false);
        self.window.focus_window();
    }

    /// 返回 true 表示事件被 egui 吃掉了(比如点在文本框里)。
    ///
    /// 尺寸变化顺手在这里同步给 painter:surface 的大小不跟着窗口走的话,下一帧要
    /// 么被拉伸要么直接 `Outdated`。
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        if let WindowEvent::Resized(size) = event {
            self.resize(size.width, size.height);
        }
        self.egui_state.on_window_event(&self.window, event).consumed
    }

    fn resize(&mut self, width: u32, height: u32) {
        // 最小化时 winit 会报 0×0,而 wgpu 不接受 0 尺寸的 surface,直接跳过。
        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            self.painter.on_window_resized(VIEWPORT, w, h);
        }
    }

    /// 画一帧,回调拿到的是 [`egui::Context`]。
    ///
    /// **新代码一律用 [`Self::redraw_ui`];这个方法仅为兼容保留。**
    ///
    /// 别指望「只要不画面板就安全」——`egui::Window` 内部就嵌着 `Resize`,首帧自适应
    /// 大小时照样触发 sizing pass,走的是下面同一条空白帧路径。安全的用法窄到不值得
    /// 记:直接用 `redraw_ui`。
    ///
    /// 原因:签名承诺的是 `FnOnce`,而 `Context::run_ui` 是个循环——任何
    /// `request_discard`(`Grid` 的首帧测量、`Resize` 的 sizing pass 都会请求)都会
    /// 让回调被再调一次,而 `FullOutput::append` 里写死了「只保留最后一趟的
    /// shapes」。`FnOnce` 只能靠 `Option::take` 兜,第二趟就没得可画,那一帧直接空
    /// 白。**画主界面请用 [`Self::redraw_ui`]**,它收的是 `FnMut`,每一趟都会重画。
    ///
    /// (另外 egui 0.36 把面板类容器 `CentralPanel` / `TopBottomPanel` / `SidePanel`
    /// 的 `show` 全改成了收 `&mut Ui`,用这里的 `&Context` 也画不了它们。)
    pub fn redraw(&mut self, ui: impl FnOnce(&egui::Context)) {
        let mut ui = Some(ui);
        self.redraw_ui(|root| {
            if let Some(ui) = ui.take() {
                ui(root.ctx());
            }
        });
    }

    /// 画一帧,回调拿到的是铺满整个窗口、没有边距也没有背景的根 [`egui::Ui`]。
    ///
    /// 回调是 `FnMut` 而不是 `FnOnce`,而且**同一帧里可能被调用多次**:egui 的
    /// `Grid`、`Resize` 这类要先量一遍才知道排版的容器会 `request_discard`,让整帧
    /// 重跑一趟(`Options::max_passes` 默认 2)。最终渲染的只有最后一趟的输出,所
    /// 以回调每趟都得老老实实把界面重新描述一遍——里面别放「只做一次」的副作用。
    pub fn redraw_ui(&mut self, ui: impl FnMut(&mut egui::Ui)) {
        // 视口信息要在取输入之前刷新:界面里 `ctx.input(|i| i.viewport())` 读到的
        // 标题 / 尺寸 / 焦点就是从这儿来的。
        egui_winit::update_viewport_info(
            &mut self.viewport_info,
            &self.egui_ctx,
            &self.window,
            false,
        );

        let mut raw_input = self.egui_state.take_egui_input(&self.window);
        raw_input.viewports.insert(VIEWPORT, self.viewport_info.clone());
        // events 是一次性的:已经随 raw_input 递给这一帧了,留着会被重复投递。
        self.viewport_info.events.clear();

        // 走 `Context::run_ui` 而不是自己 begin_pass / end_pass + 手搓根 Ui:egui
        // 默认注册了几个 plugin(label 文本选择、拖放、debug text),它们只在
        // `run_ui` 建出来的那个根 Ui 上跑,而 `Plugins` 是 pub(crate) 的,外面复刻
        // 不了。手搓根 Ui 的代价是设置界面里的文字选不中。
        let mut output = self.egui_ctx.run_ui(raw_input, ui);

        self.egui_state
            .handle_platform_output(&self.window, std::mem::take(&mut output.platform_output));

        // 视口命令(Close / Title / InnerSize / Focus …)不在 `platform_output` 里,
        // 是单独一条路;不接的话 `ctx.send_viewport_cmd(...)` 静默无效。
        let mut repaint_delay = std::time::Duration::MAX;
        if let Some(viewport) = output.viewport_output.get_mut(&VIEWPORT) {
            repaint_delay = viewport.repaint_delay;
            let commands = std::mem::take(&mut viewport.commands);
            if !commands.is_empty() {
                let mut actions = Vec::new();
                egui_winit::process_viewport_commands(
                    &self.egui_ctx,
                    &mut self.viewport_info,
                    commands,
                    &self.window,
                    &mut actions,
                );
                // 截图 / 由界面主动发起的剪切复制粘贴,设置界面都用不到;真要用得
                // 自己接,别让它悄悄没反应。
                if !actions.is_empty() {
                    tracing::debug!(count = actions.len(), "忽略未实现的视口动作请求");
                }
            }
        }
        // `ViewportCommand::Close` 只是往 `viewport_info.events` 里记一笔,真正关窗
        // 得由持有 `EguiWindow` 的那一层来做。
        if self.viewport_info.events.iter().any(|e| matches!(e, egui::ViewportEvent::Close)) {
            self.close_requested = true;
        }

        // egui 是「按需重绘」的:光标闪烁、动画、tooltip 延迟显示这类还需要下一帧的
        // 情况,靠 repaint_delay 告诉我们。0 = 立刻再来一帧;非 0 = 过这么久再来,
        // 交给事件循环去设 WaitUntil(`Duration::MAX` 表示不用再画,checked_add 会返
        // 回 None,正好落到「没有待办」)。
        if repaint_delay.is_zero() {
            self.repaint_at = None;
            self.window.request_redraw();
        } else {
            self.repaint_at = Instant::now().checked_add(repaint_delay);
        }

        // 纹理增量攒起来跨帧传:`paint_and_update_textures` 有几条早退路径(surface
        // 返回 Outdated / Lost / Occluded)不会把 delta 消费干净,而 `TexturesDelta`
        // 的 Drop 里有 `debug_assert!(is_empty())` —— 直接把每帧新出的 delta 丢过去,
        // debug 构建下拖动缩放窗口就可能 panic。改成攒在自己身上:没消费掉的部分留
        // 到下一帧继续交(那几条早退路径本来就没画出帧,egui 也会请求重绘),既不会
        // 丢字形上传,也不会有 delta 被丢弃。
        self.textures_delta.append(std::mem::take(&mut output.textures_delta));

        let primitives = self.egui_ctx.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            VIEWPORT,
            output.pixels_per_point,
            // 清屏色无所谓:界面自己会把整个 surface 铺满。
            [0.0, 0.0, 0.0, 0.0],
            &primitives,
            &mut self.textures_delta,
            Vec::new(),
            &self.window,
        );
    }
}

impl Drop for EguiWindow {
    fn drop(&mut self) {
        // 关窗口要真的把 GPU 资源还回去,而不是留着等下次开。
        //
        // `gc_viewports` 传一个空集合,等价于「没有任何视口还活着」:painter 内部
        // 的 surface / depth / msaa 纹理全部被 retain 掉。剩下的 device、queue、
        // adapter、instance 随 `self.painter` 这个字段本身析构一起还回去——painter
        // 不是 Arc 共享的,每个窗口一份,所以这里没有别的持有者。
        self.painter.gc_viewports(&egui::ViewportIdSet::default());

        // 还没上传成功的纹理增量到这里就没有意义了(要传给的那套 device 正在被销
        // 毁),显式 clear 掉——`TexturesDelta` 的 Drop 里有 debug_assert,不清就是
        // debug 构建下的一个 panic。
        self.textures_delta.clear();
    }
}
