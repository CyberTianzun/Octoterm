//! 「保存并应用」各分支的行为。全部靠假的 `Effects` 驱动,不碰真文件、
//! 不起 server、不建窗口。
//!
//! 假实现有意保留真实现的两个特征,否则测试会把 bug 一起放过去:
//! 1. 错误是**带 cause 链**的 anyhow(真实现里 `restart` / `save_config` 的错误
//!    都被 `.with_context()` 包过一层),这样才守得住「提示文案要用 `{e:#}`」;
//! 2. 每个方法的调用都记进同一条 `calls` 日志,顺序因此是可断言的事实,而不是
//!    靠两个独立的 Vec 猜出来的。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use octoterm_desktop::configfile::Editable;
use octoterm_desktop::settings::save::{apply, Applied, Current, Effects};
use octoterm_desktop::settings::state::{Form, Message};

/// `config_path()` 返回的路径。`save_config` 会校验自己收到的就是这一个 ——
/// 否则 `config_path()` 的返回值有没有被正确传下去是测不出来的。
const CONFIG_PATH: &str = "/tmp/octoterm-test/config.toml";

/// `new_token()` 的返回值。固定值,方便断言「这个 token 是现场生成的」。
const FRESH_TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// 记账用的假实现:每一步做没做过、参数是什么都留痕,失败与否由字段控制。
#[derive(Default)]
struct FakeFx {
    autostart_err: bool,
    path_err: bool,
    save_err: bool,
    restart_err: bool,
    /// restart 实际监听到的地址;None 表示「就是请求的那个」。
    actual: Option<SocketAddr>,

    /// 各方法被调用的**全局顺序**。
    calls: Vec<&'static str>,
    autostart_calls: Vec<bool>,
    /// 无论成败都记录 —— 只在成功时 push 的话,「没调用」和「调用了但失败」
    /// 就分不出来了。
    saved: Vec<Editable>,
    restarts: Vec<(SocketAddr, String)>,
}

impl Effects for FakeFx {
    fn set_autostart(&mut self, enabled: bool) -> Result<()> {
        self.calls.push("autostart");
        self.autostart_calls.push(enabled);
        if self.autostart_err {
            return Err(anyhow!("注册表拒绝访问").context("无法写入自启项"));
        }
        Ok(())
    }

    fn config_path(&mut self) -> Result<PathBuf> {
        self.calls.push("config_path");
        if self.path_err {
            return Err(anyhow!("无法确定配置目录"));
        }
        Ok(PathBuf::from(CONFIG_PATH))
    }

    fn save_config(&mut self, path: &Path, edit: &Editable) -> Result<()> {
        self.calls.push("save_config");
        assert_eq!(path, Path::new(CONFIG_PATH), "写到了 config_path() 没给过的地方");
        self.saved.push(edit.clone());
        if self.save_err {
            // 真实现是 write_atomic 的 io::Error 外面包一层 with_context,
            // 提示文案必须把两层都带出来
            return Err(anyhow!("no space left on device")
                .context(format!("无法写入 {CONFIG_PATH}.tmp")));
        }
        Ok(())
    }

    fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr> {
        self.calls.push("restart");
        self.restarts.push((listen, token));
        if self.restart_err {
            // 同上:真实现是 TcpListener::bind 的 io::Error 外面包
            // `.with_context(|| format!("无法监听 {listen}"))`
            return Err(anyhow!("Address already in use (os error 48)")
                .context(format!("无法监听 {listen}")));
        }
        Ok(self.actual.unwrap_or(listen))
    }

    fn new_token(&mut self) -> String {
        self.calls.push("new_token");
        FRESH_TOKEN.to_string()
    }
}

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn current() -> Current {
    Current { listen: Some(addr("127.0.0.1:7683")), token: Some("tok".into()), sessions: 2 }
}

/// 当前没在监听:没有地址,也没有「生效的 token」。
fn not_listening() -> Current {
    Current { listen: None, token: None, sessions: 0 }
}

fn form(host: &str, port: &str, token: &str) -> Form {
    Form { host: host.into(), port: port.into(), token: token.into(), autostart: false }
}

fn err_text(applied: &Applied) -> &str {
    match &applied.message {
        Message::Err(t) => t,
        Message::Ok(t) => panic!("期望失败,却是成功:{t}"),
    }
}

fn ok_text(applied: &Applied) -> &str {
    match &applied.message {
        Message::Ok(t) => t,
        Message::Err(t) => panic!("期望成功,却是失败:{t}"),
    }
}

#[test]
fn an_invalid_form_touches_nothing() {
    let mut fx = FakeFx::default();
    let applied = apply(&form("not-an-ip", "7683", "tok"), &current(), &mut fx);

    assert!(err_text(&applied).contains("不是合法的 IP 地址"));
    assert!(!applied.restarted);
    // 校验在最前面:连开机自启都不该被碰过
    assert!(fx.calls.is_empty(), "{:?}", fx.calls);
}

#[test]
fn an_autostart_failure_stops_before_touching_anything_else() {
    let mut fx = FakeFx { autostart_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    let text = err_text(&applied);
    assert!(text.starts_with("开机自启设置失败:"), "{text}");
    // cause 链两层都要在
    assert!(text.contains("无法写入自启项"), "{text}");
    assert!(text.contains("注册表拒绝访问"), "丢了 anyhow 的 cause:{text}");
    assert!(!applied.restarted);
    assert_eq!(fx.calls, vec!["autostart"], "自启失败后不该再做任何事");
}

#[test]
fn no_rebind_needed_means_only_writing_the_file() {
    let mut fx = FakeFx::default();
    // 地址没变、token 也没变,只是把 token 改成「不固定」不算变化
    let applied = apply(&form("127.0.0.1", "7683", ""), &current(), &mut fx);

    assert_eq!(ok_text(&applied), "已保存");
    assert!(!applied.restarted);
    assert_eq!(fx.calls, vec!["autostart", "config_path", "save_config"]);
    assert!(fx.restarts.is_empty(), "只改配置不该把用户踢下线");
    assert_eq!(fx.saved, vec![Editable { listen: addr("127.0.0.1:7683"), token: None }]);
}

#[test]
fn a_write_failure_without_rebind_is_reported_as_is() {
    let mut fx = FakeFx { save_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "7683", "tok"), &current(), &mut fx);

    let text = err_text(&applied);
    assert!(text.contains("无法写入"), "{text}");
    assert!(text.contains("no space left on device"), "丢了 anyhow 的 cause:{text}");
    assert!(!applied.restarted);
    // 调用过、只是失败了 —— 和「压根没调用」必须能区分开
    assert_eq!(fx.saved.len(), 1);
}

#[test]
fn a_changed_address_restarts_first_then_writes() {
    let mut fx = FakeFx::default();
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert_eq!(ok_text(&applied), "已生效 · 127.0.0.1:9000 · 2 个会话未受影响");
    assert!(applied.restarted);
    // 名字承诺的顺序在这里被坐实:先起来、再落盘
    assert_eq!(fx.calls, vec!["autostart", "config_path", "restart", "save_config"]);
    assert_eq!(fx.restarts, vec![(addr("127.0.0.1:9000"), "tok".to_string())]);
    assert_eq!(
        fx.saved,
        vec![Editable { listen: addr("127.0.0.1:9000"), token: Some("tok".into()) }]
    );
}

#[test]
fn the_message_reports_the_address_actually_bound() {
    let mut fx = FakeFx { actual: Some(addr("127.0.0.1:54321")), ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert!(ok_text(&applied).contains("127.0.0.1:54321"));
}

#[test]
fn an_empty_token_restarts_with_the_current_one() {
    let mut fx = FakeFx::default();
    let applied = apply(&form("127.0.0.1", "9000", ""), &current(), &mut fx);

    assert!(applied.restarted);
    // 本次运行继续用旧 token(不然保存一下自己就被踢下线),只是不再写进文件
    assert_eq!(fx.restarts, vec![(addr("127.0.0.1:9000"), "tok".to_string())]);
    assert_eq!(fx.saved, vec![Editable { listen: addr("127.0.0.1:9000"), token: None }]);
    assert!(!fx.calls.contains(&"new_token"), "有现行 token 就别乱换");
}

#[test]
fn a_write_failure_after_restart_says_applied_but_not_saved() {
    let mut fx = FakeFx { save_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    let text = err_text(&applied);
    assert!(text.contains("已在 127.0.0.1:9000 生效"), "{text}");
    assert!(text.contains("no space left on device"), "丢了 anyhow 的 cause:{text}");
    assert!(text.contains("重启后会回到旧配置"), "{text}");
    // 已经跑起来了,状态行必须跟着变
    assert!(applied.restarted);
}

#[test]
fn a_failed_restart_keeps_the_old_server_and_writes_nothing() {
    let mut fx = FakeFx { restart_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    let text = err_text(&applied);
    assert!(text.contains("无法监听 127.0.0.1:9000"), "{text}");
    assert!(text.contains("Address already in use"), "丢了 anyhow 的 cause:{text}");
    // 地址有变化 → supervisor 先 bind 后关,旧的还在跑,得说清楚
    assert!(text.contains("原服务仍在 127.0.0.1:7683 上运行"), "{text}");
    assert!(!applied.restarted);
    assert!(fx.saved.is_empty(), "没跑起来就不该把新地址写进配置");
    // 但开机自启已经改掉了:它在校验之后第一个执行,之后失败也不回滚
    assert_eq!(fx.autostart_calls, vec![false], "自启是先改的,失败不回滚");
    assert_eq!(fx.calls, vec!["autostart", "config_path", "restart"]);
}

#[test]
fn a_failed_same_address_restart_says_the_service_is_down() {
    let mut fx = FakeFx { restart_err: true, ..Default::default() };
    // 地址不变、只换 token:supervisor 会先 stop() 再 bind,失败就没有 HTTP 层了
    let applied = apply(&form("127.0.0.1", "7683", "new"), &current(), &mut fx);

    let text = err_text(&applied);
    assert!(text.contains("无法监听 127.0.0.1:7683"), "{text}");
    assert!(text.contains("服务当前未监听"), "同地址失败后服务是停着的:{text}");
}

#[test]
fn not_listening_always_tries_to_come_up() {
    let mut fx = FakeFx::default();
    let applied = apply(&form("127.0.0.1", "7683", "tok"), &not_listening(), &mut fx);

    assert!(applied.restarted);
    assert_eq!(fx.restarts, vec![(addr("127.0.0.1:7683"), "tok".to_string())]);
}

/// C1(安全):没在监听 + 表单 token 留空,以前会拿**空串**去 restart ——
/// server 侧鉴权是 `token == state.token` 的直接比较,空 token = 全部放行。
#[test]
fn a_blank_token_while_not_listening_never_yields_an_empty_one() {
    let mut fx = FakeFx::default();
    let applied = apply(&form("0.0.0.0", "7683", ""), &not_listening(), &mut fx);

    assert!(applied.restarted);
    let (_, token) = fx.restarts.first().expect("应该试着起来");
    assert!(!token.is_empty(), "空 token 会让 server 侧鉴权全部放行");
    assert_eq!(token, FRESH_TOKEN, "没有现行 token 时必须现场生成一个");
    assert!(fx.calls.contains(&"new_token"));
    // 生成出来的这个只作用于本次运行:表单说了「不固定」,文件里就不该有 token
    assert_eq!(fx.saved, vec![Editable { listen: addr("0.0.0.0:7683"), token: None }]);
}

/// 同一条路径的另一种表现形式:快照里 token 是空串(而不是 `None`)也不能漏过去。
#[test]
fn an_empty_current_token_is_not_a_token() {
    let mut fx = FakeFx::default();
    let current = Current { listen: None, token: Some(String::new()), sessions: 0 };
    apply(&form("127.0.0.1", "7683", ""), &current, &mut fx);

    let (_, token) = fx.restarts.first().expect("应该试着起来");
    assert_eq!(token, FRESH_TOKEN, "空串不是 token");
}

#[test]
fn an_unavailable_config_path_stops_everything() {
    let mut fx = FakeFx { path_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert!(err_text(&applied).contains("无法确定配置目录"));
    assert_eq!(fx.calls, vec!["autostart", "config_path"], "存不下的东西就别先应用");
    assert!(fx.saved.is_empty());
}

#[test]
fn the_autostart_checkbox_is_passed_through() {
    let mut fx = FakeFx::default();
    let mut f = form("127.0.0.1", "7683", "tok");
    f.autostart = true;
    apply(&f, &current(), &mut fx);
    assert_eq!(fx.autostart_calls, vec![true]);
}
