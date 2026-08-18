// GUI 进程不要控制台窗口
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use octoterm_desktop::app::{App, UserEvent};
use octoterm_desktop::supervisor::Supervisor;
use octoterm_desktop::{configfile, logs, single_instance};
use octoterm_server::config::Config;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let log_path = logs::init()?;

    let lock_path = configfile::default_path()?.with_file_name("octoterm.lock");
    let _guard = match single_instance::acquire(&lock_path)? {
        Some(g) => g,
        None => {
            tracing::warn!("已有 octoterm-desktop 实例在运行,退出");
            return Ok(());
        }
    };

    // 配置读不出来不是致命错误:托盘照样要出来,用户才有地方修它(见 Task 9)
    let config = Config::load(None).unwrap_or_default();
    let (token, _) = octoterm_server::config::resolve_token(None, config.token.clone());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("无法创建 tokio runtime")?;

    let mut sup = Supervisor::new(1 << 20, config.window_size, &config.launchers);
    if let Err(e) = rt.block_on(sup.restart(config.listen, token)) {
        tracing::error!(error = %e, "启动时监听失败");
    }

    let mut builder = EventLoop::<UserEvent>::with_user_event();
    // 托盘常驻应用不占 Dock、不接管菜单栏。必须在 build() 之前设,构建完再改没有效果。
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
        // 默认会在启动瞬间把前台应用踢下去抢焦点(activate_ignoring_other_apps 默认
        // true)。一个开机自启的托盘常驻应用这么干不合常规——用户当时在忙别的事,
        // 不该被 octoterm 打断,关掉这个行为。
        builder.with_activate_ignoring_other_apps(false);
    }
    let event_loop = builder.build().context("无法创建事件循环")?;
    let proxy = event_loop.create_proxy();
    // config 在 Supervisor::new 里只是被借用,这里把整份交给 App 做只读展示
    let mut app = App::new(rt, sup, proxy, log_path, config);
    event_loop.run_app(&mut app).context("事件循环异常退出")?;
    Ok(())
}
