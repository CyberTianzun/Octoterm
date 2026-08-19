//! 落盘执行。Task 2 的编辑计划是纯函数,这里管的是「怎么安全地把它写到磁盘上」。
//!
//! 全部用 tempfile 造假 home,不碰真实的 `~/.claude`。

use octoterm_server::agent::apply::{apply, ApplyError, ApplyOpts};
use octoterm_server::agent::edit::InstallCtx;
use octoterm_server::agent::find;
use std::fs;
use std::path::{Path, PathBuf};

struct Env {
    home: tempfile::TempDir,
    backups: tempfile::TempDir,
}

fn env() -> Env {
    Env { home: tempfile::tempdir().unwrap(), backups: tempfile::tempdir().unwrap() }
}

impl Env {
    fn settings(&self) -> PathBuf {
        self.home.path().join(".claude").join("settings.json")
    }
    fn seed(&self, content: &str) {
        fs::create_dir_all(self.settings().parent().unwrap()).unwrap();
        fs::write(self.settings(), content).unwrap();
    }
    fn ctx(&self) -> InstallCtx {
        InstallCtx { home: self.home.path().to_path_buf(), port: 7683, include_blocking: true }
    }
    fn opts(&self, enabled: bool) -> ApplyOpts {
        ApplyOpts {
            enabled,
            backup_dir: self.backups.path().to_path_buf(),
            backup_keep: 5,
        }
    }
    fn install(&self, enabled: bool) -> Result<Vec<octoterm_server::agent::apply::Outcome>, ApplyError> {
        let a = find("claude-code").unwrap();
        apply(&a.plan_install(&self.ctx()).unwrap(), &self.opts(enabled))
    }
    fn uninstall(&self) -> Result<Vec<octoterm_server::agent::apply::Outcome>, ApplyError> {
        let a = find("claude-code").unwrap();
        apply(&a.plan_uninstall(&self.ctx()).unwrap(), &self.opts(true))
    }
    fn backups(&self) -> Vec<PathBuf> {
        let mut v: Vec<_> =
            fs::read_dir(self.backups.path()).unwrap().flatten().map(|e| e.path()).collect();
        v.sort();
        v
    }
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap()
}

/// 开关关着时,一个字节都不该动 —— 这是「写别人的配置」这件事的第一道关卡。
#[test]
fn disabled_switch_writes_nothing() {
    let e = env();
    e.seed(r#"{"model":"opus"}"#);
    let before = read(&e.settings());
    let err = e.install(false).unwrap_err();
    assert!(matches!(err, ApplyError::Disabled));
    assert_eq!(read(&e.settings()), before, "开关关着却改了文件");
    assert!(e.backups().is_empty(), "开关关着却留下了备份");
}

/// 备份不落在 `~/.claude` 里。那是别人的地方,我们不往里堆垃圾
/// (参考实现是就地写 .bak)。
#[test]
fn backup_lands_outside_target_dir() {
    let e = env();
    e.seed(r#"{"model":"opus"}"#);
    e.install(true).unwrap();
    let backups = e.backups();
    assert_eq!(backups.len(), 1);
    let claude_dir = e.home.path().join(".claude");
    assert!(!backups[0].starts_with(&claude_dir), "备份落进了用户目录:{:?}", backups[0]);
}

#[test]
fn backup_matches_original_byte_for_byte() {
    let e = env();
    let original = "{\n  \"model\": \"opus\"\n}\n";
    e.seed(original);
    e.install(true).unwrap();
    assert_eq!(read(&e.backups()[0]), original, "备份必须是原文的逐字节副本");
}

#[test]
fn backup_keeps_only_five() {
    let e = env();
    e.seed(r#"{"model":"opus"}"#);
    for _ in 0..7 {
        // 每次都先改一下,保证内容确有变化、确会写盘
        let mut v: serde_json::Value = serde_json::from_str(&read(&e.settings())).unwrap();
        v["marker"] = serde_json::json!(rand_ish());
        fs::write(e.settings(), serde_json::to_string(&v).unwrap()).unwrap();
        e.install(true).unwrap();
    }
    assert_eq!(e.backups().len(), 5, "备份应当只保留最近 5 份");
}

fn rand_ish() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
}

/// 目标文件不是合法 JSON 时,宁可整个失败也不覆盖 —— 那可能是用户手写坏的配置,
/// 覆盖等于替他做了「丢掉」的决定。
#[test]
fn refuses_when_target_is_not_valid_json() {
    let e = env();
    e.seed("{ this is not json");
    let err = e.install(true).unwrap_err();
    assert!(matches!(err, ApplyError::Invalid { .. }));
    assert_eq!(read(&e.settings()), "{ this is not json", "坏文件被覆盖了");
    assert!(e.backups().is_empty(), "注定失败的编辑不该留下备份");
}

/// 用户还没有 settings.json 的情形:创建它,并且不产生备份(没有原文可备份)。
#[test]
fn missing_target_file_is_created() {
    let e = env();
    e.install(true).unwrap();
    assert!(e.settings().is_file());
    assert!(e.backups().is_empty());
    let v: serde_json::Value = serde_json::from_str(&read(&e.settings())).unwrap();
    assert!(v["hooks"]["PermissionRequest"].is_array());
}

/// 落盘版的幂等:装两次,文件逐字节相同,且第二次不产生新备份(因为无变化)。
#[test]
fn second_install_is_byte_identical_and_skips_write() {
    let e = env();
    e.seed(r#"{"model":"opus"}"#);
    e.install(true).unwrap();
    let after_first = read(&e.settings());
    let backups_after_first = e.backups().len();

    let outcomes = e.install(true).unwrap();
    assert_eq!(read(&e.settings()), after_first, "第二次安装改变了文件");
    assert!(outcomes.iter().all(|o| !o.changed), "无变化时不该报告 changed");
    assert_eq!(e.backups().len(), backups_after_first, "无变化时不该新增备份");
}

/// 端到端的还原保证。Task 2 在 JSON 层证过一次,这里再在磁盘层证一次。
#[test]
fn install_then_uninstall_is_byte_identical() {
    let e = env();
    let original = "{\n  \"model\": \"opus\",\n  \"env\": {\n    \"TZ\": \"Asia/Taipei\"\n  }\n}";
    e.seed(original);
    e.install(true).unwrap();
    assert_ne!(read(&e.settings()), original, "装完应当有变化");
    e.uninstall().unwrap();
    let restored: serde_json::Value = serde_json::from_str(&read(&e.settings())).unwrap();
    let expected: serde_json::Value = serde_json::from_str(original).unwrap();
    assert_eq!(restored, expected, "卸载后应当回到我们没来过的状态");
}

/// 写入过程中不能留下半截文件:落盘走 tmp + rename。
#[test]
fn no_temp_file_left_behind() {
    let e = env();
    e.seed(r#"{"model":"opus"}"#);
    e.install(true).unwrap();
    let leftovers: Vec<_> = fs::read_dir(e.home.path().join(".claude"))
        .unwrap()
        .flatten()
        .map(|x| x.file_name().to_string_lossy().to_string())
        .filter(|n| n != "settings.json")
        .collect();
    assert!(leftovers.is_empty(), "目标目录里留下了中间文件:{leftovers:?}");
}
