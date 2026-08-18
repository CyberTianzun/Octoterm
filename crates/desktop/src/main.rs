// GUI 进程不要控制台窗口
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use octoterm_desktop::app::{App, UserEvent};
use octoterm_desktop::supervisor::Supervisor;
use octoterm_desktop::{autostart, configfile, logs, single_instance};
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

    heal_autostart();

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
    }
    let event_loop = builder.build().context("无法创建事件循环")?;
    let proxy = event_loop.create_proxy();
    let mut app = App::new(rt, sup, proxy, log_path);
    event_loop.run_app(&mut app).context("事件循环异常退出")?;
    Ok(())
}

/// 自愈开机自启:`is_enabled()` 只看注册项在不在,不看它指向的可执行文件还在不在。
/// app 被移动、重命名或重装后,那条注册项就成了死链 —— 开关照样显示「已启用」,
/// 下次登录却什么都不会启动。所以每次启动都用当前 `current_exe()` 幂等地重写一遍。
/// 这一步失败不能挡住程序启动:自启只是锦上添花,记一条 warn 继续走。
fn heal_autostart() {
    match autostart::is_enabled() {
        Ok(true) => {
            if let Err(e) = autostart::set(true) {
                tracing::warn!(error = %e, "重写开机自启项失败,下次登录可能不会自动启动");
            }
        }
        Ok(false) => {}
        Err(e) => tracing::warn!(error = %e, "无法读取开机自启状态"),
    }
}
