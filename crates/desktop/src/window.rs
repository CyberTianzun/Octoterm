//! 设置窗口的载体:一个按需创建、关掉就真的销毁的 egui 窗口。
//!
//! 不用 eframe:它假定自己拥有事件循环、并在启动时就建窗口,而托盘常驻应用要的
//! 恰恰是「平时 0 窗口」。多出来的这点管线代码换的是空闲时进程里只剩 tokio 和一
//! 个状态栏图标。
//!
//! 渲染后端是 wgpu 而不是 glow:OpenGL 在 macOS 自 10.14 起已废弃,wgpu 走
//! Metal / DX12,是受支持的原生路径。

use std::num::NonZeroU32;
use std::sync::Arc;

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

        Ok(Self { painter, egui_state, egui_ctx, window })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 窗口已经开着的时候再点一次「设置…」,把它叫到前面来。
    pub fn focus(&self) {
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
    /// egui 0.36 把面板类容器(`CentralPanel` / `TopBottomPanel` / `SidePanel`)的
    /// `show` 全改成了收 `&mut Ui`,只有 `Window` / `Modal` / `Popup` 这类浮层还收
    /// `&Context`。所以真要画一个铺满窗口的界面,用下面的 [`Self::redraw_ui`]。
    pub fn redraw(&mut self, ui: impl FnOnce(&egui::Context)) {
        self.redraw_ui(|root| ui(root.ctx()));
    }

    /// 画一帧,回调拿到的是铺满整个窗口、没有边距也没有背景的根 [`egui::Ui`]。
    pub fn redraw_ui(&mut self, ui: impl FnOnce(&mut egui::Ui)) {
        let raw_input = self.egui_state.take_egui_input(&self.window);

        // 走 `Context::run_ui` 而不是自己 begin_pass / end_pass + 手搓根 Ui:egui
        // 默认注册了几个 plugin(label 文本选择、拖放、debug text),它们只在
        // `run_ui` 建出来的那个根 Ui 上跑,而 `Plugins` 是 pub(crate) 的,外面复刻
        // 不了。手搓根 Ui 的代价是设置界面里的文字选不中。
        // `run_ui` 要 FnMut,我们对外承诺的是 FnOnce,用 Option 兜一下。
        let mut ui = Some(ui);
        let mut output = self.egui_ctx.run_ui(raw_input, |root| {
            if let Some(ui) = ui.take() {
                ui(root);
            }
        });

        self.egui_state
            .handle_platform_output(&self.window, std::mem::take(&mut output.platform_output));

        let primitives = self.egui_ctx.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            VIEWPORT,
            output.pixels_per_point,
            // 清屏色无所谓:界面自己会把整个 surface 铺满。
            [0.0, 0.0, 0.0, 0.0],
            &primitives,
            &mut output.textures_delta,
            Vec::new(),
            &self.window,
        );

        // egui 是「按需重绘」的:光标闪烁、动画这类还需要下一帧的情况,靠 viewport
        // 输出里的 repaint_delay 告诉我们。不理它的话文本框光标就不闪了。
        if output
            .viewport_output
            .get(&VIEWPORT)
            .is_some_and(|v| v.repaint_delay.is_zero())
        {
            self.window.request_redraw();
        }
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
    }
}
