// GUI 进程不要控制台窗口
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use octoterm_desktop::app::{App, Startup, UserEvent};
use octoterm_desktop::supervisor::Supervisor;
use octoterm_desktop::{configfile, dialog, logs, single_instance};
use octoterm_server::config::Config;
use winit::event_loop::EventLoop;

/// 启动期弹框的统一标题。
const FAILED_TO_START: &str = "octoterm 无法启动";

/// 致命失败:先弹框把原因摆到用户面前,再把错误原样交回去。
///
/// 这几条路径以前只是让 `main` 返回 `Err` —— 也就是只打到 stderr,而这个进程恰恰
/// 没有可见的 stderr(见 [`logs`] 的模块注释)。用户双击 .app,什么都不会发生,
/// 连日志都没有:第一个可能失败的就是日志自己。Task 9 立的规矩是「启动失败不再让
/// GUI 消失」,配置坏了和端口占用有兜底,基础设施失败也得有。
///
/// 用 `{e:#}` 而不是 `{e}`:anyhow 的 Display 只有最外层那一句,用户真正需要的原因
/// (路径是哪个、是权限不足还是目录不存在)全在 cause 链里。
fn or_report<T>(title: &str, what: &str, r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        // 日志还没起来时这行是空转,起来了就多留一份底 —— 两种情况都不用特判。
        tracing::error!(error = %format!("{e:#}"), "{what}");
        dialog::fatal(title, &format!("{what}\n\n{e:#}"));
    }
    r
}

fn main() -> Result<()> {
    let log_path = or_report(FAILED_TO_START, "日志初始化失败。", logs::init())?;

    let config_path =
        or_report(FAILED_TO_START, "无法确定配置目录。", configfile::default_path())?;
    let lock_path = config_path.with_file_name("octoterm.lock");
    let lock = single_instance::acquire(&lock_path);
    let _guard = match or_report(FAILED_TO_START, "无法获取单实例锁。", lock)? {
        Some(g) => g,
        None => {
            // 这不是错误,是正常分支 —— 但必须吭一声。托盘常驻应用被重复启动是家常
            // 便饭(开机自启已经起过一份,用户又去点了一次图标),静默退出的观感就是
            // 「点了没反应」,用户只会接着再点几次。何况托盘图标在 Windows 上默认收
            // 在溢出区里,「它已经在跑了」这件事用户未必看得见。
            tracing::warn!("已有 octoterm-desktop 实例在运行,退出");
            dialog::notice(
                "octoterm 已在运行",
                "octoterm 已经在后台运行了。\n\n\
                 可以在系统托盘(macOS 是右上角菜单栏)里找到它的图标,\
                 从那里打开 Web 客户端或设置。",
            );
            return Ok(());
        }
    };

    // 配置坏了绝不能让 GUI 消失 —— 用户正是要靠这个 GUI 去修它。原因要留住,
    // 一路带进设置窗口的横幅里,否则用户只看到「设置全变回默认值了」。
    let (config, config_error) = match Config::load(None) {
        Ok(c) => (c, None),
        Err(e) => {
            tracing::error!(error = %format!("{e:#}"), "配置文件解析失败,先用默认值起来");
            (Config::default(), Some(format!("{e:#}")))
        }
    };
    // 冗余的第二道防线,不是唯一防线:`resolve_token` 现在自己就会把空白值归一成
    // 「没配」并 trim 掉前后空白,单靠它已经足够。留着这一层是因为空 token 生效的
    // 后果是全网卡无鉴权的终端(server 侧鉴权是 `token == state.token` 的字面比较,
    // 空对空一律放行),而这条路径很现实:设置界面写着「留空表示不固定」,旁边还有
    // 「打开 config.toml」按钮把用户直接引到文件里。同 `Supervisor::restart` 开头的
    // `ensure!`,纵深防御。
    let (token, _) = octoterm_server::config::resolve_token(
        None,
        config.token.clone().filter(|t| !t.trim().is_empty()),
    );

    let rt = or_report(
        FAILED_TO_START,
        "无法创建 tokio runtime。",
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("无法创建 tokio runtime"),
    )?;

    let mut sup = Supervisor::new(1 << 20, config.window_size, &config.launchers);
    // 监听失败同样不是退出的理由:端口被占用时用户要能打开设置窗口换一个端口。
    let listen_error = match rt.block_on(sup.restart(config.listen, token)) {
        Ok(addr) => {
            tracing::info!(%addr, "已监听");
            None
        }
        Err(e) => {
            // 用 `{e:#}` 而不是 `%e`:`%e` 只有最外层的 “无法监听 127.0.0.1:7683”,
            // 而用户真正需要的是里层原因(端口被占用 还是 权限不足)。托盘「查看日志」
            // 和设置窗口的横幅都靠这一份文字。
            tracing::error!(error = %format!("{e:#}"), "启动时监听失败");
            Some(format!("{e:#}"))
        }
    };

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
    let event_loop =
        or_report(FAILED_TO_START, "无法创建事件循环。", builder.build().context("无法创建事件循环"))?;
    let proxy = event_loop.create_proxy();
    // config 在 Supervisor::new 里只是被借用,这里把整份连同两条启动失败原因
    // 一起交给 App:配置做只读展示,错误做横幅 + 状态行。
    let startup = Startup { config, config_error, listen_error };
    let mut app = App::new(rt, sup, proxy, log_path, startup);
    // 事件循环异常退出时托盘也跟着没了,同样得说一声 —— 否则用户看到的是图标凭空消失。
    let ran = event_loop.run_app(&mut app).map_err(Into::into);
    or_report("octoterm 已退出", "事件循环异常退出。", ran)?;
    Ok(())
}
