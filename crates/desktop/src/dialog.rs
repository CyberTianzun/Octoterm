//! 弹给用户看的模态提示。
//!
//! GUI 进程没有可见的 stderr(见 [`crate::logs`] 的模块注释),启动期的失败如果
//! 只写日志,用户看到的就是「双击了没反应」——而最坏的一种失败恰恰是日志自己没
//! 建起来,那时候连日志都不存在。这种时候只剩弹框这一条路。
//!
//! 这里的函数都是同步模态的:弹着的时候调用线程整个卡住。启动期本来就没有别的
//! 事要做,卡住是对的。

/// 报一条致命错误。调用方在这之后应当结束进程 —— 不要报完继续跑。
pub fn fatal(title: &str, body: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(title)
        .set_description(body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// 报一条中性提示,不代表出错。
pub fn notice(title: &str, body: &str) {
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(title)
        .set_description(body)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
