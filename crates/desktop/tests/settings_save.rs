//! 「保存并应用」五个分支的行为。全部靠假的 `Effects` 驱动,不碰真文件、
//! 不起 server、不建窗口。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use octoterm_desktop::configfile::Editable;
use octoterm_desktop::settings::save::{apply, Applied, Current, Effects};
use octoterm_desktop::settings::state::{Form, Message};

/// 记账用的假实现:每一步做没做过、参数是什么都留痕,失败与否由字段控制。
#[derive(Default)]
struct FakeFx {
    autostart_err: bool,
    path_err: bool,
    save_err: bool,
    restart_err: bool,
    /// restart 实际监听到的地址;None 表示「就是请求的那个」。
    actual: Option<SocketAddr>,

    autostart_calls: Vec<bool>,
    saved: Vec<Editable>,
    restarts: Vec<(SocketAddr, String)>,
}

impl Effects for FakeFx {
    fn set_autostart(&mut self, enabled: bool) -> Result<()> {
        self.autostart_calls.push(enabled);
        if self.autostart_err {
            return Err(anyhow!("没有权限"));
        }
        Ok(())
    }

    fn config_path(&mut self) -> Result<PathBuf> {
        if self.path_err {
            return Err(anyhow!("无法确定配置目录"));
        }
        Ok(PathBuf::from("/tmp/octoterm-test/config.toml"))
    }

    fn save_config(&mut self, _path: &Path, edit: &Editable) -> Result<()> {
        if self.save_err {
            return Err(anyhow!("磁盘满了"));
        }
        self.saved.push(edit.clone());
        Ok(())
    }

    fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr> {
        self.restarts.push((listen, token.clone()));
        if self.restart_err {
            return Err(anyhow!("无法监听 {listen}"));
        }
        Ok(self.actual.unwrap_or(listen))
    }
}

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn current() -> Current {
    Current { listen: Some(addr("127.0.0.1:7683")), token: "tok".into(), sessions: 2 }
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
    assert!(fx.autostart_calls.is_empty());
    assert!(fx.saved.is_empty());
    assert!(fx.restarts.is_empty());
}

#[test]
fn an_autostart_failure_stops_before_touching_anything_else() {
    let mut fx = FakeFx { autostart_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert!(err_text(&applied).starts_with("开机自启设置失败:"));
    assert!(!applied.restarted);
    assert!(fx.saved.is_empty());
    assert!(fx.restarts.is_empty(), "自启失败后不该动 HTTP 层");
}

#[test]
fn no_rebind_needed_means_only_writing_the_file() {
    let mut fx = FakeFx::default();
    // 地址没变、token 也没变,只是把 token 改成「不固定」不算变化
    let applied = apply(&form("127.0.0.1", "7683", ""), &current(), &mut fx);

    assert_eq!(ok_text(&applied), "已保存");
    assert!(!applied.restarted);
    assert!(fx.restarts.is_empty(), "只改配置不该把用户踢下线");
    assert_eq!(fx.saved, vec![Editable { listen: addr("127.0.0.1:7683"), token: None }]);
}

#[test]
fn a_write_failure_without_rebind_is_reported_as_is() {
    let mut fx = FakeFx { save_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "7683", "tok"), &current(), &mut fx);

    assert!(err_text(&applied).contains("磁盘满了"));
    assert!(!applied.restarted);
}

#[test]
fn a_changed_address_restarts_first_then_writes() {
    let mut fx = FakeFx::default();
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert_eq!(ok_text(&applied), "已生效 · 127.0.0.1:9000 · 2 个会话未受影响");
    assert!(applied.restarted, "调用方要据此刷新托盘状态行");
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
}

#[test]
fn a_write_failure_after_restart_says_applied_but_not_saved() {
    let mut fx = FakeFx { save_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    let text = err_text(&applied);
    assert!(text.contains("已在 127.0.0.1:9000 生效"), "{text}");
    assert!(text.contains("重启后会回到旧配置"), "{text}");
    // 已经跑起来了,状态行必须跟着变
    assert!(applied.restarted);
}

#[test]
fn a_failed_restart_leaves_everything_untouched() {
    let mut fx = FakeFx { restart_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert!(err_text(&applied).contains("无法监听 127.0.0.1:9000"));
    assert!(!applied.restarted);
    assert!(fx.saved.is_empty(), "没跑起来就不该把新地址写进配置");
}

#[test]
fn not_listening_always_tries_to_come_up() {
    let mut fx = FakeFx::default();
    let current = Current { listen: None, token: String::new(), sessions: 0 };
    let applied = apply(&form("127.0.0.1", "7683", "tok"), &current, &mut fx);

    assert!(applied.restarted);
    assert_eq!(fx.restarts, vec![(addr("127.0.0.1:7683"), "tok".to_string())]);
}

#[test]
fn an_unavailable_config_path_stops_everything() {
    let mut fx = FakeFx { path_err: true, ..Default::default() };
    let applied = apply(&form("127.0.0.1", "9000", "tok"), &current(), &mut fx);

    assert!(err_text(&applied).contains("无法确定配置目录"));
    assert!(fx.restarts.is_empty(), "存不下的东西就别先应用");
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
