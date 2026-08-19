//! 新建会话时可选的启动项(launcher)从哪来。
//!
//! 一条 launcher = 一个可以直接拿去 spawn 的 argv(+ 可选 cwd)+ 一个给人看的
//! 名字。它们由若干 **provider** 提供,provider 是这里唯一的扩展点:
//!
//! - `builtin` —— 系统默认 shell,永远存在,永远排第一;
//! - `config`  —— 用户在 octoterm 自己的 config.toml 里写的 `[[launcher]]`;
//! - `iterm2` / `windows-terminal` —— **只读**扫描系统上已装终端的配置,把它们
//!   已经配好的 profile 直接拿来用。
//!
//! **发现只读是硬约束**:launcher 从不写别人的配置文件,也不要求用户在 octoterm 里
//! 把配置再抄一遍。这样"配置"这件事的成本是零,而扩展新来源只需要再实现一个
//! [`LauncherProvider`]。
//!
//! 这条约束的作用域是**发现**。集成(`crate::agent`)是另一回事:在用户显式动作下,
//! 它会写 agent 的配置文件去装 hook,受开关门控、写前备份、卸载能还原。两者的边界
//! 见 `docs/superpowers/specs/2026-08-18-octoterm-agent-integration-design.md`。
//!
//! provider 的失败是**局部的**:任何一个 provider 抛错(配置文件损坏、权限不足、
//! 格式变了)只会让它自己的条目消失并留下一条日志,不影响其他 provider —— 新建
//! 会话这个动作绝不能因为某个第三方终端的配置坏了就用不了。

use serde::Serialize;

pub mod builtin;
pub mod cmdline;
pub mod iterm2;
pub mod jsonc;
pub mod user_config;
pub mod windows_terminal;

use crate::config::LauncherSpec;

/// 单个 provider 最多贡献多少条。防止一份异常的配置文件(或手滑写出的巨大
/// `[[launcher]]` 列表)把菜单撑爆。
const PER_PROVIDER_CAP: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Launcher {
    /// `<provider>:<provider 内部的稳定标识>`。同一台机器上跨进程重启保持稳定,
    /// 这样客户端可以记住"上次用的那个"。
    pub id: String,
    pub provider: &'static str,
    /// 显示名。来自 profile 自己的名字,可能重复,不作为标识使用。
    pub name: String,
    /// 一行命令预览,给 UI 显示用。对 Windows Terminal 这类"原文是一整行命令行"
    /// 的来源,这里是**原文**,比把切分后的 argv 再拼回去更贴近用户的记忆。
    pub detail: String,
    pub command: Vec<String>,
    pub cwd: Option<String>,
}

impl Launcher {
    /// `detail` 直接由 argv 拼出的常见情形。
    pub fn new(
        provider: &'static str,
        local_id: &str,
        name: impl Into<String>,
        command: Vec<String>,
    ) -> Self {
        let detail = command.join(" ");
        Self {
            id: format!("{provider}:{local_id}"),
            provider,
            name: name.into(),
            detail,
            command,
            cwd: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn with_cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd;
        self
    }
}

pub trait LauncherProvider: Send + Sync {
    /// 稳定的 provider 标识,同时是 [`Launcher::id`] 的前缀。
    fn id(&self) -> &'static str;
    /// 扫描一次。允许返回空列表(该来源在本机没装/没配),Err 表示扫描本身失败。
    fn discover(&self) -> anyhow::Result<Vec<Launcher>>;
}

/// 本机可用的 provider 列表,按菜单里的展示顺序排列。
///
/// 顺序即优先级:内置默认排第一(它一定能用),用户自己写的排第二(用户显式表达
/// 的意图应该压过扫描出来的结果),扫描来的排最后。
pub fn providers(specs: &[LauncherSpec]) -> Vec<Box<dyn LauncherProvider>> {
    let mut list: Vec<Box<dyn LauncherProvider>> = vec![Box::new(builtin::Builtin)];
    if !specs.is_empty() {
        list.push(Box::new(user_config::UserConfig::new(specs.to_vec())));
    }
    #[cfg(target_os = "macos")]
    list.push(Box::new(iterm2::ITerm2::new()));
    #[cfg(windows)]
    list.push(Box::new(windows_terminal::WindowsTerminal::new()));
    list
}

/// 跑一遍所有 provider,合并结果。
///
/// 每次调用都真的去读一次文件,不做缓存:这个动作只在用户打开"新建"菜单时发生
/// (量级是几毫秒的几次文件读),而缓存会让"刚在 iTerm2 里加了个 profile"要等到
/// 重启 octoterm 才生效 —— 对一个随手用的工具来说,这个代价换不来什么。
pub fn discover_all(providers: &[Box<dyn LauncherProvider>]) -> Vec<Launcher> {
    let mut out: Vec<Launcher> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for p in providers {
        match p.discover() {
            Ok(items) => {
                let total = items.len();
                for l in items.into_iter().take(PER_PROVIDER_CAP) {
                    if seen.insert(l.id.clone()) {
                        out.push(l);
                    }
                }
                if total > PER_PROVIDER_CAP {
                    tracing::warn!(
                        provider = p.id(),
                        total,
                        cap = PER_PROVIDER_CAP,
                        "launcher provider 条目过多,已截断"
                    );
                }
            }
            Err(e) => {
                // 局部失败:记下来,继续下一个 provider。
                tracing::warn!(provider = p.id(), error = %e, "launcher provider 扫描失败,已跳过");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, Vec<Launcher>);
    impl LauncherProvider for Fixed {
        fn id(&self) -> &'static str {
            self.0
        }
        fn discover(&self) -> anyhow::Result<Vec<Launcher>> {
            Ok(self.1.clone())
        }
    }

    struct Broken;
    impl LauncherProvider for Broken {
        fn id(&self) -> &'static str {
            "broken"
        }
        fn discover(&self) -> anyhow::Result<Vec<Launcher>> {
            anyhow::bail!("配置文件坏了")
        }
    }

    fn l(provider: &'static str, id: &str) -> Launcher {
        Launcher::new(provider, id, id, vec!["sh".into()])
    }

    #[test]
    fn broken_provider_does_not_take_down_the_others() {
        let ps: Vec<Box<dyn LauncherProvider>> =
            vec![Box::new(Broken), Box::new(Fixed("a", vec![l("a", "x")]))];
        let out = discover_all(&ps);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a:x");
    }

    #[test]
    fn duplicate_ids_keep_the_first_provider_wins() {
        let ps: Vec<Box<dyn LauncherProvider>> = vec![
            Box::new(Fixed("a", vec![Launcher::new("a", "x", "先来的", vec!["sh".into()])])),
            Box::new(Fixed("a", vec![Launcher::new("a", "x", "后来的", vec!["sh".into()])])),
        ];
        let out = discover_all(&ps);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "先来的");
    }

    #[test]
    fn per_provider_cap_truncates() {
        let many = (0..PER_PROVIDER_CAP + 10).map(|i| l("a", &i.to_string())).collect();
        let ps: Vec<Box<dyn LauncherProvider>> = vec![Box::new(Fixed("a", many))];
        assert_eq!(discover_all(&ps).len(), PER_PROVIDER_CAP);
    }

    #[test]
    fn builtin_is_always_first_and_never_empty() {
        let ps = providers(&[]);
        assert_eq!(ps[0].id(), "builtin");
        let out = discover_all(&ps);
        assert!(!out.is_empty(), "至少要有内置默认 shell");
        assert_eq!(out[0].provider, "builtin");
    }
}
