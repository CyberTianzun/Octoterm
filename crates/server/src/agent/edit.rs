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
#[derive(Clone)]
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
    ///
    /// 带的是整个 group 而不只是 hook 本身:各家的 group 形状不一样(Claude Code 有
    /// `matcher`,Codex 的实际配置里没有),这个差异属于 adapter,不属于这里。
    EnsureHook { event: String, group: Value },
    /// 删掉该事件下所有我方 hook;被我们清空的组一并删掉,事件键空了也删掉。
    RemoveOurs { event: String },
    /// 清空:扫遍 `hooks` 下的**每一个**事件,删掉所有我方 hook,不看端口、不看
    /// 事件名在不在当前的托管清单里、也不看 URL 里写的是哪家 agent。
    ///
    /// 和 `RemoveOurs` 的区别正是它存在的理由:卸载只认「这个版本会装的那些事件」,
    /// 旧版本装过、后来不再托管的事件,或者异常中断留下的半截条目,卸载够不着。
    /// 「是不是我们的」仍然走同一个极严的判定(`is_ours_any_port`),所以清得再狠
    /// 也碰不到用户自己的 hook。
    PurgeOurs,
    /// 整个文件归我们管:写成 `content`。
    ///
    /// 给那些没有「hook 配置」、只认「往某目录放一个文件」的 agent(pi 的扩展)。
    /// 所有权靠文件开头的 [`OWNED_MARKER`] 认 —— 同名但没有标记的文件是用户的,
    /// 拒绝覆盖(`apply` 回 `Invalid`)。
    WriteOwnedFile { content: String },
    /// 删掉我们写的那个文件。没有标记就不删,文件不在就什么都不做。
    RemoveOwnedFile,
    /// 共用配置文件里,整个顶层键归我们管:写成 `value`。
    ///
    /// 给那些「顶层是具名 hook、底下才是事件」的配置(Antigravity 的
    /// `~/.gemini/config/hooks.json`,几个 Antigravity 产品共用,不能整份接管)。
    /// 所有权靠内容认:同名键底下只要有一条命令不是我们的,就是用户的,拒绝覆盖。
    SetOwnedKey { key: String, value: Value },
    /// 删掉我们的那个顶层键。不是我们的、或不存在,就什么都不做。
    RemoveOwnedKey { key: String },
}

/// 整文件托管时写在第一行的所有权标记。改它等于让所有旧安装变成「不是我们的」,
/// 卸载和清空都会够不着 —— 不要改。
pub const OWNED_MARKER: &str = "octoterm-managed";

/// 这段文本是不是我们整文件托管写出来的:只看第一行。
pub fn is_owned_file(text: &str) -> bool {
    text.lines().next().is_some_and(|l| l.contains(OWNED_MARKER))
}

impl EditOp {
    /// 整文件操作走 `apply` 的另一条落盘路径(不解析 JSON)。
    pub fn is_file_op(&self) -> bool {
        matches!(self, Self::WriteOwnedFile { .. } | Self::RemoveOwnedFile)
    }

    /// 「清空」时对同一个文件该做什么。整文件 / 整键托管的删掉那一份;按事件挂 hook 的
    /// 扫遍所有事件(见 [`EditOp::PurgeOurs`])。
    pub fn purge_op(&self) -> EditOp {
        match self {
            Self::WriteOwnedFile { .. } | Self::RemoveOwnedFile => Self::RemoveOwnedFile,
            Self::SetOwnedKey { key, .. } | Self::RemoveOwnedKey { key } => Self::RemoveOwnedKey { key: key.clone() },
            Self::EnsureHook { .. } | Self::RemoveOurs { .. } | Self::PurgeOurs => Self::PurgeOurs,
        }
    }
}

/// 这一块配置是不是我们写的:底下至少有一条命令,而且**每一条**都是我们的
/// (不看端口)。混着一条别人的命令就不是 —— 宁可拒绝覆盖,也不能吞掉用户的 hook。
pub fn is_owned_value(v: &Value) -> bool {
    let mut commands = Vec::new();
    collect_hook_handlers(v, &mut commands);
    !commands.is_empty() && commands.iter().all(|h| is_ours_any_port(h))
}

/// 一块配置里所有带 `command` 的处理器对象(任意深度)。
pub fn collect_hook_handlers<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            if map.get("command").is_some_and(Value::is_string) {
                out.push(v);
            }
            for child in map.values() {
                collect_hook_handlers(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|c| collect_hook_handlers(c, out)),
        _ => {}
    }
}

pub fn apply_to_json(doc: &mut Value, op: &EditOp) -> Result<()> {
    match op {
        EditOp::EnsureHook { event, group } => ensure(doc, event, group),
        EditOp::RemoveOurs { event } => {
            remove_ours(doc, event);
            Ok(())
        }
        EditOp::WriteOwnedFile { .. } | EditOp::RemoveOwnedFile => {
            anyhow::bail!("整文件操作不能应用到 JSON 上")
        }
        EditOp::SetOwnedKey { key, value } => {
            let obj = doc.as_object_mut().context("配置根不是 JSON 对象")?;
            if let Some(existing) = obj.get(key)
                && !is_owned_value(existing)
            {
                anyhow::bail!("`{key}` 已经存在,而且不是 octoterm 写的,拒绝覆盖");
            }
            obj.insert(key.clone(), value.clone());
            Ok(())
        }
        EditOp::RemoveOwnedKey { key } => {
            if let Some(obj) = doc.as_object_mut()
                && obj.get(key).is_some_and(is_owned_value)
            {
                // shift_remove:保持其余键的顺序,理由见 `remove_ours`
                obj.shift_remove(key);
            }
            Ok(())
        }
        EditOp::PurgeOurs => {
            let events: Vec<String> = doc
                .get("hooks")
                .and_then(Value::as_object)
                .map(|h| h.keys().cloned().collect())
                .unwrap_or_default();
            for event in events {
                remove_ours(doc, &event);
            }
            Ok(())
        }
    }
}

fn ensure(doc: &mut Value, event: &str, group: &Value) -> Result<()> {
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
    groups.push(group.clone());
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

/// 这份配置是不是已经和「按当前服务装一遍」的结果一致。只读,不改 `doc`。
///
/// 回答的是启动自检要的那个问题:**装是装了,可它还对得上这个服务吗?** 端口变了、
/// 新版本多托管了一个事件、Codex 那条命令里的二进制路径换了,都算对不上。
///
/// 故意不用「把计划应用一遍再比文本 / 比整棵树」:`ensure` 会把我们的组挪到数组
/// 末尾,别的程序在我们之后追加了 hook 就会被误报成不一致。这里只看语义 ——
///
/// - 每条 `EnsureHook`:该事件下我方 hook **恰好一条**,且和计划写的一模一样;
/// - 整份文件里没有任何一条我方 hook 指向别的端口(那是打不通的残留)。
pub fn is_current(doc: &Value, edits: &[&EditOp], port: u16) -> bool {
    let ensured = edits.iter().all(|op| match op {
        EditOp::EnsureHook { event, group } => {
            let want = group.get("hooks").and_then(Value::as_array).and_then(|l| l.first());
            let ours: Vec<&Value> = hooks_in(doc, event).filter(|h| is_ours_any_port(h)).collect();
            matches!((ours.as_slice(), want), ([only], Some(w)) if *only == w)
        }
        EditOp::SetOwnedKey { key, value } => doc.get(key) == Some(value),
        // 计划里只有「装」才有「对不对得上」可言
        EditOp::RemoveOurs { .. } | EditOp::PurgeOurs | EditOp::RemoveOwnedKey { .. } => true,
        // 整文件托管不在 JSON 里,由调用方直接比文本(见 `agent::install_is_current`)
        EditOp::WriteOwnedFile { .. } | EditOp::RemoveOwnedFile => true,
    });
    let no_strays = doc
        .get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|h| h.keys())
        .flat_map(|event| hooks_in(doc, event))
        .all(|h| our_url(h).is_none_or(|u| u.port == port));
    ensured && no_strays
}

fn hooks_in<'a>(doc: &'a Value, event: &str) -> impl Iterator<Item = &'a Value> {
    doc.get("hooks")
        .and_then(|h| h.get(event))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|g| g.get("hooks"))
        .filter_map(Value::as_array)
        .flatten()
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
///
/// 两种承载都认:`type: "http"` 的 `url` 字段(Claude Code),和 `type: "command"` 的
/// 命令串里出现的同一个 URL(Codex —— 它不支持 http 型 hook,只能由我们自己的二进制
/// 转发)。一个所有权概念,两种传输。
fn our_url(hook: &Value) -> Option<OurUrl> {
    let raw = match hook.get("type").and_then(Value::as_str)? {
        "http" => hook.get("url").and_then(Value::as_str)?.to_string(),
        "command" => url_in_command(hook.get("command").and_then(Value::as_str)?)?,
        _ => return None,
    };
    let url = raw.as_str();
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

/// 从命令串里挑出我们的 URL。命令串形如
/// `"/path/to/octoterm-server" hook http://127.0.0.1:7683/hook/codex/stop`,
/// 按空白切开找那个 token 就够了 —— URL 里不会有空白。
pub(crate) fn url_in_command(cmd: &str) -> Option<String> {
    let decoded = super::hook_cli::decode_windows_command(cmd);
    let cmd = decoded.as_deref().unwrap_or(cmd);
    cmd.split_whitespace()
        .map(|t| t.trim_matches(|c| c == '"' || c == '\''))
        .find(|t| t.starts_with("http://127.0.0.1:"))
        .map(str::to_string)
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
