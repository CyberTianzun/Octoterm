//! 配置编辑**计划**,以及「这条 hook 是不是我们写的」的判定。
//!
//! 这里的函数全是纯函数:进出都是 `serde_json::Value`,不碰文件系统。落盘是
//! `apply.rs` 的事。这么切有三个好处,任何一条都够:
//!
//! 1. 装之前可以先把 diff 摆给用户看 —— 我们要改的是**用户的** `settings.json`;
//! 2. 幂等是免费的:计划先摘干净再写入,应用两次必然收敛,不需要事后去重;
//! 3. 单测不碰真实 home,断言一份 JSON 比断言文件系统副作用容易一个量级。
//!
//! 参考实现 clawd-on-desk 正是因为早期直接写盘,产生了**字节相同**的重复条目,
//! 事后再也无法用命令串谓词区分「留这个删那些」,只好写一个按位置折叠的函数来收拾。

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

/// 生成计划所需的上下文。
pub struct InstallCtx {
    pub home: PathBuf,
    /// 当前监听端口。只影响**新写入**的 URL;删除时不看端口(见 `is_ours_any_port`)。
    pub port: u16,
    /// 装不装**决策类**(阻塞式)hook。
    ///
    /// 实测:同一事件上多个阻塞 hook 会全部触发,**最后注册的赢**。我们把自己的组
    /// append 在数组末尾,所以装上就会**覆盖别家的决策**(本机同时装了 clawd-on-desk
    /// 时就会发生)。不偷偷占先:发现别家的阻塞式 hook 时默认置 false,只装遥测类,
    /// 由用户看过冲突说明后显式打开。
    pub include_blocking: bool,
}

pub struct ConfigEdit {
    pub path: PathBuf,
    pub op: EditOp,
}

#[derive(Debug, Clone)]
pub enum EditOp {
    /// 保证该事件下**恰好有一条**我方 hook,其余条目原样不动。
    EnsureHook { event: String, spec: Value },
    /// 删掉该事件下所有我方 hook;被我们清空的组一并删掉,事件键空了也删掉。
    RemoveOurs { event: String },
}

pub fn apply_to_json(doc: &mut Value, op: &EditOp) -> Result<()> {
    match op {
        EditOp::EnsureHook { event, spec } => ensure(doc, event, spec),
        EditOp::RemoveOurs { event } => {
            remove_ours(doc, event);
            Ok(())
        }
    }
}

fn ensure(doc: &mut Value, event: &str, spec: &Value) -> Result<()> {
    // 先摘干净再写入 —— 幂等就是从这一行来的
    remove_ours(doc, event);
    let obj = doc.as_object_mut().context("配置根不是 JSON 对象")?;
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("`hooks` 不是 JSON 对象")?;
    let groups = hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .with_context(|| format!("`hooks.{event}` 不是数组"))?;
    groups.push(json!({ "matcher": "", "hooks": [spec] }));
    Ok(())
}

fn remove_ours(doc: &mut Value, event: &str) {
    let Some(root) = doc.as_object_mut() else { return };
    let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) else { return };
    let Some(groups) = hooks.get_mut(event).and_then(Value::as_array_mut) else { return };

    // 只删「**被我们**清空的」组。原本就空的组是用户的,留着 —— 我们没往里放过
    // 东西,就没有资格把它拿走。
    let mut emptied = Vec::new();
    for (i, group) in groups.iter_mut().enumerate() {
        let Some(list) = group.get_mut("hooks").and_then(Value::as_array_mut) else { continue };
        let before = list.len();
        list.retain(|h| !is_ours_any_port(h));
        if list.len() != before && list.is_empty() {
            emptied.push(i);
        }
    }
    for i in emptied.into_iter().rev() {
        groups.remove(i);
    }

    // 空数组 / 空对象是残渣,不留:卸载后文件要看不出我们来过。
    //
    // **必须用 `shift_remove` 而不是 `remove`**:开了 `preserve_order` 之后
    // `Map` 底下是 `IndexMap`,而 `remove` 是 **swap_remove** 语义 —— 它把最后一个
    // 键换到被删的位置,顺序当场就乱了。这正是我们开 `preserve_order` 想避免的事,
    // 用错方法等于白开。由 `second_install_is_byte_identical_and_skips_write` 抓出。
    if groups.is_empty() {
        hooks.shift_remove(event);
    }
    if hooks.is_empty() {
        root.shift_remove("hooks");
    }
}

/// 这条 hook 是不是我们在**当前端口**上写的。
///
/// 判定必须极严:宁可漏删自己的,也绝不误删用户自建的审批端点。参考实现里同名的
/// `isManagedPermissionUrl()` 要求协议、主机、路径、端口全部命中才认领,这里同等。
pub fn is_ours(hook: &Value, port: u16) -> bool {
    our_url(hook).is_some_and(|u| u.port == port)
}

/// 是不是我们写的 —— **不看端口**。
///
/// 卸载和「先摘再写」都用这个:用户改过监听端口之后,旧端口的条目仍然是我们的
/// 垃圾,必须能清掉。反过来,`is_ours` 看端口,是为了把「装过但端口对不上」这种
/// 「装了却不生效」的状态报给用户。
fn is_ours_any_port(hook: &Value) -> bool {
    our_url(hook).is_some()
}

struct OurUrl {
    port: u16,
}

/// 只认 `http://127.0.0.1:<port>/hook/<agent>/<event>`,且不带 query / fragment /
/// 凭据。任何一处不符都不是我们的。
fn our_url(hook: &Value) -> Option<OurUrl> {
    if hook.get("type").and_then(Value::as_str) != Some("http") {
        return None;
    }
    let url = hook.get("url").and_then(Value::as_str)?;
    if url.contains('?') || url.contains('#') || url.contains('@') {
        return None;
    }
    let rest = url.strip_prefix("http://127.0.0.1:")?;
    let (port, path) = rest.split_once('/')?;
    let port: u16 = port.parse().ok()?;
    let slugs: Vec<&str> = path.split('/').collect();
    let [hook_seg, agent, event] = slugs[..] else { return None };
    if hook_seg != "hook" || !is_slug(agent) || !is_slug(event) {
        return None;
    }
    Some(OurUrl { port })
}

fn is_slug(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `SessionStart` → `session-start`。URL 里只允许小写与连字符(见 `is_slug`),
/// 所以事件名要转写;转写必须是双射,否则回调路由认不出是哪个事件。
pub fn slug_of_event(event: &str) -> String {
    let mut out = String::with_capacity(event.len() + 4);
    for (i, c) in event.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i != 0 {
                out.push('-');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `session-start` → `SessionStart`。`slug_of_event` 的逆,回调路由用它把路径段
/// 还原成 agent 的事件名。
pub fn event_of_slug(slug: &str) -> String {
    slug.split('-')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                Some(f) => f.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_roundtrips() {
        for e in ["SessionStart", "PermissionRequest", "Stop", "PreToolUse"] {
            assert_eq!(event_of_slug(&slug_of_event(e)), e, "转写必须是双射");
        }
    }
}
