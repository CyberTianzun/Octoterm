//! Windows Terminal 的 profile。
//!
//! 只读,不写。两层来源叠在一起:
//! - 用户的 `settings.json`(路径见 [`settings_paths`])
//! - 安装程序丢在 `Fragments/{source}/*.json` 里的 JSON fragment(路径见
//!   [`fragment_roots`])
//!
//! 动态 profile(新版 WSL、ESP-IDF 等)在 settings 里通常只有 guid / source /
//! 用户覆盖,**没有 commandline**。真正的命令行在 fragment 里,guid 是两边的
//! 主键。fragment 没写 guid 时,按 WT 的规则用 `{source, name}` 算 UUID v5。
//!
//! 解析([`parse`])是平台无关的纯函数,路径发现才按平台 gate —— 否则这套逻辑
//! 只能在 Windows 上测,而它恰恰是最容易被 schema 变动咬到的地方。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;
use uuid::Uuid;

use super::{cmdline, jsonc, Launcher, LauncherProvider};

pub const ID: &str = "windows-terminal";

/// WSL 的**旧** profile 由 WT 内置生成器产出,`source` 是这个值,配置里没有
/// commandline。规则稳定(`wsl.exe -d <name>`),值得代劳。新版 WSL 改走
/// fragment,`source` 是 `Microsoft.WSL`,命令行在 fragment 里,不再猜。
///
/// 其余没有 commandline 的内置生成器(PowerShell Core、Azure Cloud Shell、
/// Visual Studio)无从推断,直接跳过。
const WSL_SOURCE: &str = "Windows.Terminal.Wsl";

/// WT 给 fragment / 第三方插件算 guid 用的命名空间。
/// 见 https://learn.microsoft.com/windows/terminal/json-fragment-extensions
const FRAGMENT_NAMESPACE: Uuid = Uuid::from_u128(0xf65ddb7e_706b_4499_8a50_40313caf510a);

pub struct WindowsTerminal;

impl WindowsTerminal {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsTerminal {
    fn default() -> Self {
        Self::new()
    }
}

impl LauncherProvider for WindowsTerminal {
    fn id(&self) -> &'static str {
        ID
    }

    fn discover(&self) -> Result<Vec<Launcher>> {
        let env = |k: &str| std::env::var(k).ok();
        let is_file = |p: &str| Path::new(p).is_file();
        let fragments = read_fragment_files(&fragment_roots());
        for path in settings_paths() {
            // 没装 / 没这个版本:不是错误,换下一个候选
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            tracing::debug!(path = %path.display(), "读取 Windows Terminal 配置");
            return parse(&src, &fragments, &env, &is_file);
        }
        // settings.json 还没生成(装了 fragment 但没开过 WT)时,fragment 自己
        // 就够拼出菜单,别空手返回。
        if fragments.is_empty() {
            return Ok(Vec::new());
        }
        parse("{}", &fragments, &env, &is_file)
    }
}

/// settings.json 的候选位置,按优先级。存在多个时只用第一个命中的。
#[cfg(windows)]
pub fn settings_paths() -> Vec<PathBuf> {
    let Ok(local) = std::env::var("LOCALAPPDATA") else {
        return Vec::new();
    };
    let local = PathBuf::from(local);
    vec![
        local.join(r"Packages\Microsoft.WindowsTerminal_8wekyb3d8bbwe\LocalState\settings.json"),
        local.join(
            r"Packages\Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe\LocalState\settings.json",
        ),
        // 非应用商店(portable / MSI)安装
        local.join(r"Microsoft\Windows Terminal\settings.json"),
    ]
}

#[cfg(not(windows))]
pub fn settings_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// Fragment 根目录。先全机(`ProgramData`)再当前用户,同 guid 时后者覆盖前者,
/// 跟 WT 自己的加载顺序一致。
#[cfg(windows)]
pub fn fragment_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(pd) = std::env::var("ProgramData") {
        roots.push(PathBuf::from(pd).join(r"Microsoft\Windows Terminal\Fragments"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(local).join(r"Microsoft\Windows Terminal\Fragments"));
    }
    roots
}

#[cfg(not(windows))]
pub fn fragment_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// 一份 fragment 文件:目录名就是 settings 里的 `source`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FragmentFile {
    pub source: String,
    pub contents: String,
}

/// 扫 `Fragments/{source}/*.json`。单个文件读失败只记日志,不让整个 provider
/// 归零。目录和文件都排一下序,菜单顺序才确定。
pub fn read_fragment_files(roots: &[PathBuf]) -> Vec<FragmentFile> {
    let mut out = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut apps: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        apps.sort();
        for app_dir in apps {
            let source = app_dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if source.is_empty() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&app_dir) else {
                continue;
            };
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file() && is_json_file(p))
                .collect();
            files.sort();
            for path in files {
                match std::fs::read_to_string(&path) {
                    Ok(contents) => out.push(FragmentFile {
                        source: source.clone(),
                        contents,
                    }),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %e, "Windows Terminal fragment 读取失败")
                    }
                }
            }
        }
    }
    out
}

fn is_json_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// fragment 没写 guid 时,WT 用命名空间 + `source` + `name` 的 UTF-16LE 字节
/// 算 UUID v5。返回带花括号的小写形式,跟 settings.json 里的写法一致。
pub fn fragment_profile_guid(source: &str, name: &str) -> String {
    let app_ns = Uuid::new_v5(&FRAGMENT_NAMESPACE, &utf16le(source));
    let id = Uuid::new_v5(&app_ns, &utf16le(name));
    format!("{{{id}}}")
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn guid_key(s: &str) -> String {
    let s = s.trim();
    let inner = s.strip_prefix('{').unwrap_or(s);
    let inner = inner.strip_suffix('}').unwrap_or(inner).trim();
    format!("{{{}}}", inner.to_ascii_lowercase())
}

/// fragment / defaults / 用户桩叠出来的字段。后 apply 的覆盖先 apply 的。
#[derive(Debug, Default, Clone)]
struct Overlay {
    source: Option<String>,
    name: Option<String>,
    commandline: Option<String>,
    starting_directory: Option<String>,
    hidden: Option<bool>,
}

impl Overlay {
    fn from_json(v: &Value, source: Option<String>) -> Self {
        Self {
            source,
            name: str_field(v, "name").map(str::to_string),
            commandline: str_field(v, "commandline").map(str::to_string),
            starting_directory: str_field(v, "startingDirectory").map(str::to_string),
            hidden: v.get("hidden").and_then(Value::as_bool),
        }
    }

    fn apply(&mut self, other: &Self) {
        if let Some(v) = other.source.clone() {
            // source 是身份,只在还没有的时候收下,避免 `updates` 把被改的
            // profile 改写成 fragment 自己的目录名
            if self.source.is_none() {
                self.source = Some(v);
            }
        }
        if let Some(v) = other.name.clone() {
            self.name = Some(v);
        }
        if let Some(v) = other.commandline.clone() {
            self.commandline = Some(v);
        }
        if let Some(v) = other.starting_directory.clone() {
            self.starting_directory = Some(v);
        }
        if let Some(v) = other.hidden {
            self.hidden = Some(v);
        }
    }
}

#[derive(Default)]
struct Generated {
    by_guid: HashMap<String, Overlay>,
    /// 首次见到的 created profile,决定 fragment-only 条目的顺序
    order: Vec<String>,
    updates: HashMap<String, Vec<Overlay>>,
}

fn profiles_list(root: &Value) -> (Option<Value>, Vec<Value>) {
    match root.get("profiles") {
        Some(Value::Array(list)) => (None, list.clone()),
        Some(obj @ Value::Object(_)) => (
            obj.get("defaults").cloned(),
            obj.get("list")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
        ),
        _ => (None, Vec::new()),
    }
}

fn ingest_fragments(files: &[FragmentFile]) -> Generated {
    let mut generated = Generated::default();
    for f in files {
        let Ok(root) = serde_json::from_str::<Value>(&jsonc::strip(&f.contents)) else {
            continue;
        };
        let (_, list) = profiles_list(&root);
        for p in &list {
            if !p.is_object() {
                continue;
            }
            if let Some(target) = str_field(p, "updates") {
                let key = guid_key(target);
                if key == "{}" {
                    continue;
                }
                // updates 只改目标 profile 的字段,不带走自己的 source
                generated.updates
                    .entry(key)
                    .or_default()
                    .push(Overlay::from_json(p, None));
                continue;
            }
            let name = str_field(p, "name");
            let key = match str_field(p, "guid") {
                Some(g) => guid_key(g),
                None => match name {
                    Some(n) => guid_key(&fragment_profile_guid(&f.source, n)),
                    None => continue,
                },
            };
            let overlay = Overlay::from_json(p, Some(f.source.clone()));
            if !generated.by_guid.contains_key(&key) {
                generated.order.push(key.clone());
            }
            generated.by_guid.entry(key).or_default().apply(&overlay);
        }
    }
    generated
}

fn disabled_sources(root: &Value) -> HashSet<String> {
    root.get("disabledProfileSources")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn lookup<'a>(generated: &'a Generated, key: &str) -> (Option<&'a Overlay>, &'a [Overlay]) {
    let base = generated.by_guid.get(key);
    let updates = generated.updates.get(key).map(Vec::as_slice).unwrap_or(&[]);
    (base, updates)
}

fn merge_layers(defaults: &Overlay, generated: &Generated, key: Option<&str>, top: Overlay) -> Overlay {
    let mut merged = defaults.clone();
    if let Some(k) = key {
        let (base, updates) = lookup(generated, k);
        if let Some(base) = base {
            merged.apply(base);
        }
        for u in updates {
            merged.apply(u);
        }
    }
    merged.apply(&top);
    merged
}

/// `~` 是 WT 给 WSL 的"发行版 home",不是 Windows 路径;带过去 spawn 只会
/// 发现目录不存在再回落。干脆当成没写。
fn normalize_cwd(raw: Option<&str>, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let expanded = cmdline::expand_windows_env(raw?, env);
    let t = expanded.trim();
    if t.is_empty() || t == "~" || t.starts_with("~/") || t.starts_with("~\\") {
        return None;
    }
    Some(expanded)
}

fn to_argv(
    raw: &str,
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&str) -> bool,
) -> Option<(Vec<String>, String)> {
    let argv = cmdline::split_windows(&cmdline::expand_windows_env(raw, env));
    if argv.is_empty() {
        return None;
    }
    Some((
        cmdline::glue_unquoted_windows_exe(argv, is_file),
        raw.to_string(),
    ))
}

fn emit(
    local_id: &str,
    merged: &Overlay,
    disabled: &HashSet<String>,
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&str) -> bool,
) -> Option<Launcher> {
    if merged.hidden.unwrap_or(false) {
        return None;
    }
    if let Some(src) = merged.source.as_deref() {
        if disabled.contains(src) {
            return None;
        }
    }
    let name = merged.name.as_deref()?;
    let (command, detail) = if let Some(raw) = merged.commandline.as_deref() {
        to_argv(raw, env, is_file)?
    } else if merged.source.as_deref() == Some(WSL_SOURCE) {
        let argv = vec!["wsl.exe".to_string(), "-d".to_string(), name.to_string()];
        let detail = argv.join(" ");
        (argv, detail)
    } else {
        return None;
    };
    let cwd = normalize_cwd(merged.starting_directory.as_deref(), env);
    Some(
        Launcher::new(ID, local_id, name, command)
            .with_detail(detail)
            .with_cwd(cwd),
    )
}

/// 解析 settings.json,并用 fragment 补上动态 profile 缺的 commandline。
/// `env` 用来展开 `%VAR%`,`is_file` 用来把没加引号的 `C:\Program Files\...`
/// 粘回一个 argv[0] —— 都注入进来是为了可测。
pub fn parse(
    src: &str,
    fragments: &[FragmentFile],
    env: &dyn Fn(&str) -> Option<String>,
    is_file: &dyn Fn(&str) -> bool,
) -> Result<Vec<Launcher>> {
    let root: Value = serde_json::from_str(&jsonc::strip(src))?;
    let (defaults, list) = profiles_list(&root);
    let default_overlay = defaults
        .as_ref()
        .map(|d| Overlay::from_json(d, None))
        .unwrap_or_default();
    let disabled = disabled_sources(&root);
    let generated = ingest_fragments(fragments);

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for p in &list {
        if !p.is_object() {
            continue;
        }
        let user = Overlay::from_json(p, str_field(p, "source").map(str::to_string));
        let key = str_field(p, "guid").map(guid_key).or_else(|| {
            let source = user.source.as_deref()?;
            let name = user.name.as_deref()?;
            Some(guid_key(&fragment_profile_guid(source, name)))
        });
        if let Some(ref k) = key {
            seen.insert(k.clone());
        }
        let merged = merge_layers(&default_overlay, &generated, key.as_deref(), user);
        let Some(local_id) = str_field(p, "guid").or(merged.name.as_deref()) else {
            continue;
        };
        if let Some(l) = emit(local_id, &merged, &disabled, env, is_file) {
            out.push(l);
        }
    }

    // settings 还没写桩的新 fragment,按加载顺序接在后面
    for k in &generated.order {
        if !seen.insert(k.clone()) {
            continue;
        }
        let user = Overlay::default();
        let merged = merge_layers(&default_overlay, &generated, Some(k), user);
        if let Some(l) = emit(k, &merged, &disabled, env, is_file) {
            out.push(l);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<String> {
        match k {
            "SystemRoot" => Some(r"C:\Windows".into()),
            "USERPROFILE" => Some(r"C:\Users\hiro".into()),
            _ => None,
        }
    }

    fn parse_sample(src: &str) -> Vec<Launcher> {
        parse(src, &[], &env, &|_| false).unwrap()
    }

    fn parse_with(src: &str, fragments: &[FragmentFile]) -> Vec<Launcher> {
        parse(src, fragments, &env, &|_| false).unwrap()
    }

    fn frag(source: &str, contents: &str) -> FragmentFile {
        FragmentFile {
            source: source.into(),
            contents: contents.into(),
        }
    }

    const SAMPLE: &str = r#"
    {
        // Windows Terminal 自己生成的配置就带注释
        "defaultProfile": "{guid-ps}",
        "profiles":
        {
            "defaults": { "startingDirectory": "%USERPROFILE%" },
            "list":
            [
                {
                    "guid": "{guid-cmd}",
                    "name": "命令提示符",
                    "commandline": "%SystemRoot%\\System32\\cmd.exe",
                    "startingDirectory": "C:\\work",
                },
                {
                    "guid": "{guid-ps7}",
                    "name": "PowerShell 7",
                    "commandline": "\"C:\\Program Files\\PowerShell\\7\\pwsh.exe\" -NoLogo"
                },
                {
                    "guid": "{guid-git}",
                    "name": "Git Bash",
                    "commandline": "C:\\Program Files\\Git\\bin\\bash.exe -li"
                },
                {
                    "guid": "{guid-wsl}",
                    "name": "Ubuntu",
                    "source": "Windows.Terminal.Wsl"
                },
                {
                    "guid": "{guid-hidden}",
                    "name": "藏起来的",
                    "commandline": "cmd.exe",
                    "hidden": true
                },
                {
                    "guid": "{guid-azure}",
                    "name": "Azure Cloud Shell",
                    "source": "Windows.Terminal.Azure"
                }
            ]
        }
    }"#;

    #[test]
    fn parses_profiles_with_env_and_quoting() {
        let out = parse_sample(SAMPLE);
        let ids: Vec<&str> = out.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "windows-terminal:{guid-cmd}",
                "windows-terminal:{guid-ps7}",
                "windows-terminal:{guid-git}",
                "windows-terminal:{guid-wsl}"
            ],
            "hidden 的和无法推断命令的应该被跳过"
        );

        let cmd = &out[0];
        assert_eq!(cmd.name, "命令提示符");
        assert_eq!(cmd.command, [r"C:\Windows\System32\cmd.exe"]);
        assert_eq!(cmd.cwd.as_deref(), Some(r"C:\work"));
        // detail 保留原文,用户在 WT 里看到的就是这个
        assert_eq!(cmd.detail, r"%SystemRoot%\System32\cmd.exe");

        // 带空格的程序路径不能被拆开
        assert_eq!(
            out[1].command,
            [r"C:\Program Files\PowerShell\7\pwsh.exe", "-NoLogo"]
        );
        // defaults 的 startingDirectory 在自己没写时生效
        assert_eq!(out[1].cwd.as_deref(), Some(r"C:\Users\hiro"));
    }

    #[test]
    fn wsl_profiles_get_a_synthesized_command() {
        let out = parse_sample(SAMPLE);
        let wsl = out.iter().find(|l| l.name == "Ubuntu").unwrap();
        assert_eq!(wsl.command, ["wsl.exe", "-d", "Ubuntu"]);
    }

    #[test]
    fn unquoted_git_bash_path_is_not_split_on_spaces() {
        // 真机上的 Git Bash profile 就是这样写的,一个引号都没有。
        // is_file 认这个路径,粘完必须是一个 argv[0],否则 CreateProcessW 去找 C:\Program。
        let exists = |p: &str| p.eq_ignore_ascii_case(r"C:\Program Files\Git\bin\bash.exe");
        let out = parse(SAMPLE, &[], &env, &exists).unwrap();
        let git = out.iter().find(|l| l.name == "Git Bash").unwrap();
        assert_eq!(git.command, [r"C:\Program Files\Git\bin\bash.exe", "-li"]);
        // detail 仍是 WT 里的原文,方便对照
        assert_eq!(git.detail, r"C:\Program Files\Git\bin\bash.exe -li");
    }

    #[test]
    fn accepts_the_legacy_array_shaped_profiles_key() {
        let out = parse_sample(
            r#"{ "profiles": [ { "guid": "{g}", "name": "老格式", "commandline": "cmd.exe" } ] }"#,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].command, ["cmd.exe"]);
    }

    #[test]
    fn missing_or_empty_profiles_is_not_an_error() {
        assert!(parse_sample("{}").is_empty());
        assert!(parse_sample(r#"{"profiles": {"list": []}}"#).is_empty());
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse("{ not json", &[], &env, &|_| false).is_err());
    }

    #[test]
    fn fragment_guid_matches_documented_examples() {
        // 文档里的 Git Bash,以及本机 ESP-IDF 安装器写进 settings 的 guid
        assert_eq!(
            fragment_profile_guid("Git", "Git Bash"),
            "{2ece5bfe-50ed-5f3a-ab87-5cd4baafed2b}"
        );
        assert_eq!(
            fragment_profile_guid("ESP-IDF 5.5", "ESP-IDF 5.5"),
            "{68813744-91af-523a-83df-82adcac75a91}"
        );
    }

    #[test]
    fn fragment_fills_commandline_for_settings_stub() {
        // 安装器只写 name + commandline,guid 由 WT 按 source+name 算出来
        let settings = r#"{
            "profiles": { "list": [
                {
                    "guid": "{68813744-91af-523a-83df-82adcac75a91}",
                    "hidden": false,
                    "name": "ESP-IDF 5.5",
                    "source": "ESP-IDF 5.5"
                }
            ]}
        }"#;
        let fragments = [frag(
            "ESP-IDF 5.5",
            r#"{
                "profiles": [{
                    "name": "ESP-IDF 5.5",
                    "startingDirectory": "D:/Espressif/frameworks/esp-idf-v5.5.3/",
                    "commandline": "C:\\WINDOWS/System32/WindowsPowerShell/v1.0/powershell.exe -ExecutionPolicy Bypass -NoExit -File D:\\Espressif/Initialize-Idf.ps1 -IdfId esp-idf-fabfeda26c56c35d8c56d67988cf4834"
                }]
            }"#,
        )];
        let out = parse_with(settings, &fragments);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].id,
            "windows-terminal:{68813744-91af-523a-83df-82adcac75a91}"
        );
        assert_eq!(out[0].name, "ESP-IDF 5.5");
        assert_eq!(
            out[0].command,
            [
                r"C:\WINDOWS/System32/WindowsPowerShell/v1.0/powershell.exe",
                "-ExecutionPolicy",
                "Bypass",
                "-NoExit",
                "-File",
                r"D:\Espressif/Initialize-Idf.ps1",
                "-IdfId",
                "esp-idf-fabfeda26c56c35d8c56d67988cf4834"
            ]
        );
        assert_eq!(
            out[0].cwd.as_deref(),
            Some("D:/Espressif/frameworks/esp-idf-v5.5.3/")
        );
        assert!(out[0].detail.contains("Initialize-Idf.ps1"));
    }

    #[test]
    fn wsl_fragment_uses_distribution_id_not_synthesized_dash_d() {
        let settings = r#"{
            "profiles": { "list": [
                {
                    "font": { "size": 10 },
                    "guid": "{ee603799-d7fe-5a9f-a662-e784d8a502df}",
                    "hidden": false,
                    "name": "Ubuntu 26.04",
                    "source": "Microsoft.WSL"
                }
            ]}
        }"#;
        let fragments = [frag(
            "Microsoft.WSL",
            r#"{
                "profiles": [
                    { "hidden": true, "updates": "{2c4de342-38b7-51cf-b940-2309a097f518}" },
                    {
                        "commandline": "C:\\WINDOWS\\system32\\wsl.exe --distribution-id {fc3bf427-722c-4c2b-84bf-17202d2e3740}",
                        "guid": "{ee603799-d7fe-5a9f-a662-e784d8a502df}",
                        "name": "Ubuntu",
                        "startingDirectory": "~"
                    }
                ]
            }"#,
        )];
        let out = parse_with(settings, &fragments);
        assert_eq!(out.len(), 1);
        // 用户改过的名字压过 fragment 原文
        assert_eq!(out[0].name, "Ubuntu 26.04");
        assert_eq!(
            out[0].command,
            [
                r"C:\WINDOWS\system32\wsl.exe",
                "--distribution-id",
                "{fc3bf427-722c-4c2b-84bf-17202d2e3740}"
            ]
        );
        // `~` 不是 Windows 路径,不能当成 cwd
        assert_eq!(out[0].cwd, None);
    }

    #[test]
    fn user_commandline_wins_over_fragment() {
        let settings = r#"{
            "profiles": { "list": [
                {
                    "guid": "{68813744-91af-523a-83df-82adcac75a91}",
                    "name": "ESP-IDF 5.5",
                    "source": "ESP-IDF 5.5",
                    "commandline": "cmd.exe /k echo hi"
                }
            ]}
        }"#;
        let fragments = [frag(
            "ESP-IDF 5.5",
            r#"{ "profiles": [{ "name": "ESP-IDF 5.5", "commandline": "powershell.exe" }] }"#,
        )];
        let out = parse_with(settings, &fragments);
        assert_eq!(out[0].command, ["cmd.exe", "/k", "echo", "hi"]);
        assert_eq!(out[0].detail, "cmd.exe /k echo hi");
    }

    #[test]
    fn fragment_only_profile_is_appended() {
        let fragments = [frag(
            "ESP-IDF 5.5",
            r#"{ "profiles": [{ "name": "ESP-IDF 5.5", "commandline": "powershell.exe" }] }"#,
        )];
        let out = parse_with(r#"{ "profiles": { "list": [] } }"#, &fragments);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].id,
            "windows-terminal:{68813744-91af-523a-83df-82adcac75a91}"
        );
        assert_eq!(out[0].command, ["powershell.exe"]);
    }

    #[test]
    fn fragment_updates_can_hide_an_existing_profile() {
        let settings = r#"{
            "profiles": { "list": [
                { "guid": "{2c4de342-38b7-51cf-b940-2309a097f518}", "name": "Ubuntu", "source": "Windows.Terminal.Wsl" }
            ]}
        }"#;
        let fragments = [frag(
            "Microsoft.WSL",
            r#"{ "profiles": [{ "hidden": true, "updates": "{2c4de342-38b7-51cf-b940-2309a097f518}" }] }"#,
        )];
        assert!(parse_with(settings, &fragments).is_empty());
    }

    #[test]
    fn user_hidden_false_unhides_a_fragment() {
        let settings = r#"{
            "profiles": { "list": [
                { "guid": "{g}", "name": "X", "source": "App", "hidden": false }
            ]}
        }"#;
        let fragments = [frag(
            "App",
            r#"{ "profiles": [{ "guid": "{g}", "name": "X", "commandline": "cmd.exe", "hidden": true }] }"#,
        )];
        let out = parse_with(settings, &fragments);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].command, ["cmd.exe"]);
    }

    #[test]
    fn disabled_profile_sources_are_skipped() {
        let settings = r#"{
            "disabledProfileSources": ["Microsoft.WSL"],
            "profiles": { "list": [
                { "guid": "{ee603799-d7fe-5a9f-a662-e784d8a502df}", "name": "Ubuntu", "source": "Microsoft.WSL" }
            ]}
        }"#;
        let fragments = [frag(
            "Microsoft.WSL",
            r#"{ "profiles": [{ "guid": "{ee603799-d7fe-5a9f-a662-e784d8a502df}", "name": "Ubuntu", "commandline": "wsl.exe" }] }"#,
        )];
        assert!(parse_with(settings, &fragments).is_empty());
    }

    #[test]
    fn guid_match_is_case_insensitive() {
        let settings = r#"{
            "profiles": { "list": [
                { "guid": "{EE603799-D7FE-5A9F-A662-E784D8A502DF}", "name": "Ubuntu", "source": "Microsoft.WSL" }
            ]}
        }"#;
        let fragments = [frag(
            "Microsoft.WSL",
            r#"{ "profiles": [{ "guid": "{ee603799-d7fe-5a9f-a662-e784d8a502df}", "commandline": "wsl.exe" }] }"#,
        )];
        let out = parse_with(settings, &fragments);
        assert_eq!(out.len(), 1);
        // 对外的 id 保持 settings 里的写法,客户端记过的不会漂
        assert_eq!(
            out[0].id,
            "windows-terminal:{EE603799-D7FE-5A9F-A662-E784D8A502DF}"
        );
        assert_eq!(out[0].command, ["wsl.exe"]);
    }

    #[test]
    fn later_fragment_overlays_earlier_for_the_same_guid() {
        let fragments = [
            frag(
                "A",
                r#"{ "profiles": [{ "guid": "{g}", "name": "A", "commandline": "a.exe" }] }"#,
            ),
            frag(
                "A",
                r#"{ "profiles": [{ "guid": "{g}", "commandline": "b.exe" }] }"#,
            ),
        ];
        let out = parse_with("{}", &fragments);
        assert_eq!(out[0].command, ["b.exe"]);
        assert_eq!(out[0].name, "A");
    }

    #[test]
    fn malformed_fragment_is_skipped_not_fatal() {
        let fragments = [
            frag("Bad", "{ not json"),
            frag(
                "Good",
                r#"{ "profiles": [{ "name": "Ok", "commandline": "cmd.exe" }] }"#,
            ),
        ];
        let out = parse_with("{}", &fragments);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Ok");
    }

    #[test]
    fn settings_list_order_is_preserved_before_fragment_only() {
        let settings = r#"{
            "profiles": { "list": [
                { "guid": "{one}", "name": "One", "commandline": "one.exe" }
            ]}
        }"#;
        let fragments = [frag(
            "Two",
            r#"{ "profiles": [{ "name": "Two", "commandline": "two.exe" }] }"#,
        )];
        let out = parse_with(settings, &fragments);
        assert_eq!(
            out.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["One", "Two"]
        );
    }

    #[test]
    fn read_fragment_files_walks_source_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Fragments");
        let app = root.join("ESP-IDF 5.5");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("fragment.json"), r#"{"profiles":[]}"#).unwrap();
        std::fs::write(app.join("readme.txt"), "nope").unwrap();
        let files = read_fragment_files(&[root]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].source, "ESP-IDF 5.5");
        assert!(files[0].contents.contains("profiles"));
    }

    /// 这台机器上如果装了 WSL / ESP-IDF 的 fragment,discover 必须能拼出命令,
    /// 而不是继续跳过 settings 里的桩。没装就当跳过。
    #[cfg(windows)]
    #[test]
    fn discover_fills_real_machine_fragment_stubs_when_present() {
        let local = match std::env::var("LOCALAPPDATA") {
            Ok(v) => PathBuf::from(v),
            Err(_) => return,
        };
        let root = local.join(r"Microsoft\Windows Terminal\Fragments");
        let has_wsl = root.join(r"Microsoft.WSL").is_dir();
        let has_idf = root.join(r"ESP-IDF 5.5").is_dir();
        if !has_wsl && !has_idf {
            return;
        }
        let out = WindowsTerminal::new().discover().unwrap();
        if has_wsl {
            let wsl = out
                .iter()
                .find(|l| l.command.iter().any(|a| a.contains("wsl.exe")));
            let wsl = wsl.expect("Microsoft.WSL fragment should produce a launcher");
            assert!(
                wsl.command
                    .iter()
                    .any(|a| a.contains("--distribution-id") || a == "-d"),
                "{:?}",
                wsl.command
            );
        }
        if has_idf {
            let idf = out.iter().find(|l| l.name.contains("ESP-IDF 5.5"));
            let idf = idf.expect("ESP-IDF 5.5 fragment should produce a launcher");
            assert!(
                idf.command
                    .iter()
                    .any(|a| a.contains("Initialize-Idf") || a.contains("powershell")),
                "{:?}",
                idf.command
            );
        }
    }
}
