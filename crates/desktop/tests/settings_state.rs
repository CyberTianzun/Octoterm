use octoterm_desktop::configfile::Editable;
use octoterm_desktop::settings::state::{needs_rebind, FieldError, Form};

fn form(host: &str, port: &str, token: &str) -> Form {
    Form { host: host.into(), port: port.into(), token: token.into(), autostart: false }
}

#[test]
fn a_valid_form_produces_an_editable() {
    let got = form("127.0.0.1", "7683", "abc").validate().unwrap();
    assert_eq!(got.listen.to_string(), "127.0.0.1:7683");
    assert_eq!(got.token.as_deref(), Some("abc"));
}

#[test]
fn an_empty_token_means_not_pinned() {
    let got = form("127.0.0.1", "7683", "   ").validate().unwrap();
    assert_eq!(got.token, None, "空白 token 等于不固定");
}

#[test]
fn a_bad_host_is_a_host_error() {
    assert!(matches!(form("not-an-ip", "7683", "").validate(), Err(FieldError::Host(_))));
}

#[test]
fn ipv6_hosts_are_accepted() {
    let got = form("::1", "7683", "").validate().unwrap();
    assert_eq!(got.listen.to_string(), "[::1]:7683");
}

#[test]
fn port_zero_and_garbage_are_port_errors() {
    assert!(matches!(form("127.0.0.1", "0", "").validate(), Err(FieldError::Port(_))));
    assert!(matches!(form("127.0.0.1", "abc", "").validate(), Err(FieldError::Port(_))));
    assert!(matches!(form("127.0.0.1", "70000", "").validate(), Err(FieldError::Port(_))));
}

#[test]
fn from_current_round_trips() {
    let listen = "0.0.0.0:9000".parse().unwrap();
    let f = Form::from_current(listen, true);
    assert_eq!(f.host, "0.0.0.0");
    assert_eq!(f.port, "9000");
    assert!(f.autostart);
    assert_eq!(f.validate().unwrap().listen, listen);
}

#[test]
fn rebind_only_when_listen_or_token_actually_changes() {
    let current = "127.0.0.1:7683".parse().unwrap();
    let same = Editable { listen: current, token: Some("live".into()) };
    assert!(!needs_rebind(current, "live", &same));

    let new_port = Editable { listen: "127.0.0.1:9000".parse().unwrap(), token: Some("live".into()) };
    assert!(needs_rebind(current, "live", &new_port));

    let new_token = Editable { listen: current, token: Some("other".into()) };
    assert!(needs_rebind(current, "live", &new_token));
}

#[test]
fn unpinning_the_token_does_not_rebind() {
    // 清空 token 只是把键从 config.toml 里拿掉,本次运行仍用现有 token —— 否则
    // 用户会毫无预兆地被踢下线。
    let current = "127.0.0.1:7683".parse().unwrap();
    let unpinned = Editable { listen: current, token: None };
    assert!(!needs_rebind(current, "live", &unpinned));
}
