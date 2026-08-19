//! 配置编辑计划与所有权判定。
//!
//! 这是整个 agent 集成里**唯一会改用户文件**的逻辑,所以测试密度最高。三条验收线:
//! 幂等、不碰别人的、卸载能还原。

use octoterm_server::agent::edit::{apply_to_json, is_ours, ConfigEdit, InstallCtx};
use octoterm_server::agent::find;
use serde_json::{json, Value};

const PORT: u16 = 7683;

fn ctx() -> InstallCtx {
    InstallCtx { home: "/home/u".into(), port: PORT, include_blocking: true }
}

fn run(doc: &mut Value, edits: Vec<ConfigEdit>) {
    for e in edits {
        apply_to_json(doc, &e.op).unwrap();
    }
}

fn installed(base: Value) -> Value {
    let mut doc = base;
    run(&mut doc, find("claude-code").unwrap().plan_install(&ctx()).unwrap());
    doc
}

fn uninstalled(base: Value) -> Value {
    let mut doc = base;
    run(&mut doc, find("claude-code").unwrap().plan_uninstall(&ctx()).unwrap());
    doc
}

#[test]
fn install_is_idempotent() {
    let once = installed(json!({}));
    let twice = installed(once.clone());
    assert_eq!(once, twice, "同一份计划应用两次必须收敛");
}

#[test]
fn install_creates_permission_hook() {
    let doc = installed(json!({}));
    let groups = doc["hooks"]["PermissionRequest"].as_array().expect("要有 PermissionRequest");
    let hook = &groups[0]["hooks"][0];
    assert_eq!(hook["type"], "http");
    assert_eq!(hook["url"], format!("http://127.0.0.1:{PORT}/hook/claude-code/permission-request"));
    assert_eq!(hook["timeout"], 600, "阻塞式决策要给足人类反应时间");
    assert!(hook.get("async").is_none(), "决策类 hook 不能是 async,否则拿不到回答");
    assert_eq!(hook["headers"]["Authorization"], "Bearer $OCTOTERM_HOOK_TOKEN");
    assert_eq!(hook["allowedEnvVars"][0], "OCTOTERM_HOOK_TOKEN");
}

#[test]
fn telemetry_hooks_are_async_and_short() {
    let doc = installed(json!({}));
    let hook = &doc["hooks"]["Stop"][0]["hooks"][0];
    assert_eq!(hook["async"], true, "遥测不能拖住 Claude");
    assert_eq!(hook["timeout"], 5);
}

#[test]
fn install_keeps_user_hooks() {
    let user = json!({"hooks":{"Stop":[{"matcher":"","hooks":[
        {"type":"command","command":"my-own-script.sh"}]}]}});
    let out = installed(user);
    let kept = out["hooks"]["Stop"]
        .as_array()
        .unwrap()
        .iter()
        .any(|g| g["hooks"][0]["command"] == "my-own-script.sh");
    assert!(kept, "用户自己的 hook 必须原样保留");
}

#[test]
fn uninstall_restores_original() {
    let user = json!({"model":"opus","hooks":{"Stop":[{"matcher":"","hooks":[
        {"type":"command","command":"my-own-script.sh"}]}]}});
    let out = uninstalled(installed(user.clone()));
    assert_eq!(out, user, "卸载必须能还原到我们没来过的状态");
}

#[test]
fn uninstall_drops_empty_event_keys() {
    let out = uninstalled(installed(json!({})));
    assert_eq!(out, json!({}), "卸载后不能留下空的 hooks 残渣");
}

#[test]
fn uninstall_on_clean_doc_is_noop() {
    assert_eq!(uninstalled(json!({"model":"opus"})), json!({"model":"opus"}));
}

/// 所有权判定必须**极严**:长得像但不是我们的,一律不认领。宁可漏删自己的,
/// 也绝不误删用户自建的审批端点。
#[test]
fn ownership_is_strict() {
    let mine = json!({"type":"http","url":format!("http://127.0.0.1:{PORT}/hook/claude-code/stop")});
    assert!(is_ours(&mine, PORT), "自己写的必须认得出来");

    let bad = [
        ("协议不同", json!({"type":"http","url":format!("https://127.0.0.1:{PORT}/hook/claude-code/stop")})),
        ("主机不同", json!({"type":"http","url":format!("http://127.0.0.2:{PORT}/hook/claude-code/stop")})),
        ("端口不同", json!({"type":"http","url":"http://127.0.0.1:9999/hook/claude-code/stop"})),
        ("带 query", json!({"type":"http","url":format!("http://127.0.0.1:{PORT}/hook/claude-code/stop?x=1")})),
        ("带 fragment", json!({"type":"http","url":format!("http://127.0.0.1:{PORT}/hook/claude-code/stop#a")})),
        ("带凭据", json!({"type":"http","url":format!("http://u:p@127.0.0.1:{PORT}/hook/claude-code/stop")})),
        ("别家端点", json!({"type":"http","url":format!("http://127.0.0.1:{PORT}/permission")})),
        ("路径多一段", json!({"type":"http","url":format!("http://127.0.0.1:{PORT}/hook/claude-code/stop/x")})),
        ("不是 http 型", json!({"type":"command","command":format!("curl 127.0.0.1:{PORT}/hook/claude-code/stop")})),
        ("没有 url", json!({"type":"http"})),
    ];
    for (why, v) in bad {
        assert!(!is_ours(&v, PORT), "不该认领({why}): {v}");
    }
}

/// clawd-on-desk 在同一台机器上时,PermissionRequest 上会有它的阻塞式 hook。
/// 那不是我们的,不能删 —— 但必须能被报出来,因为两个阻塞 hook 抢同一个事件
/// 是真实的互操作问题。
#[test]
fn foreign_blocking_hook_is_not_ours() {
    let clawd = json!({"type":"http","url":"http://127.0.0.1:23333/permission","timeout":600});
    assert!(!is_ours(&clawd, PORT));
}

/// 真实世界的形状:一份已经被**别的厂商**装满 hook 的 `settings.json`
/// (15 个事件、14 条 command hook + 1 条指向别家端口的阻塞式 http hook),
/// 外加 env / permissions / model 等用户自己的内容。
///
/// 合成用例覆盖不到的是「多事件 × 多组 × 混合类型」同时出现时的组清理逻辑 ——
/// 那正是最容易把别人的条目一起带走的地方。
mod real_world {
    use super::*;

    fn fixture() -> Value {
        let raw = include_str!("fixtures/claude-settings-with-other-vendor.json");
        serde_json::from_str(raw).unwrap()
    }

    #[test]
    fn install_then_uninstall_restores_byte_for_byte() {
        let before = fixture();
        let after = uninstalled(installed(before.clone()));
        assert_eq!(after, before, "在别家装满的配置上装完再卸,必须一字不差地还原");
    }

    #[test]
    fn install_is_idempotent_on_real_shape() {
        let once = installed(fixture());
        assert_eq!(installed(once.clone()), once);
    }

    #[test]
    fn other_vendor_hooks_survive_install() {
        let out = installed(fixture());
        let count = |doc: &Value| -> usize {
            doc["hooks"]
                .as_object()
                .unwrap()
                .values()
                .flat_map(|g| g.as_array().unwrap())
                .flat_map(|g| g["hooks"].as_array().unwrap())
                .filter(|h| {
                    h["command"].as_str().is_some_and(|c| c.contains("other-vendor-hook.js"))
                        || h["url"].as_str() == Some("http://127.0.0.1:23333/permission")
                })
                .count()
        };
        assert_eq!(count(&out), count(&fixture()), "别家的 hook 一条都不能少");
    }

    /// key 顺序必须保住 —— 我们改的是用户的文件,把人家的 env/permissions/model
    /// 按字典序重排一遍是不可接受的副作用(serde_json 的 preserve_order 特性)。
    #[test]
    fn preserves_user_key_order() {
        let out = installed(fixture());
        let keys: Vec<&str> = out.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["env", "permissions", "model", "hooks"]);
    }
}
