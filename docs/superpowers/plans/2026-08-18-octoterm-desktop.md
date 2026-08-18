# octoterm desktop 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加 `octoterm-desktop` —— 一个 Windows/macOS 托盘常驻程序,内嵌 octoterm server,提供托盘菜单与一个原生设置窗口。

**Architecture:** 主线程跑 winit 事件循环(托盘 + 按需创建的 egui 窗口),后台线程跑 tokio runtime 承载内嵌 server。`SessionManager` 由 `Supervisor` 长期持有,改配置时只重建 listener 与 `AppState`,pty 会话零损失。所有可测逻辑(配置读写、表单校验、server 生命周期)与 GUI 完全隔离。

**Tech Stack:** Rust 2024 / winit 0.30 / egui 0.36 + egui-winit + egui-wgpu / tray-icon 0.24 / toml_edit / tokio / axum(经 octoterm-server)

设计依据:`docs/superpowers/specs/2026-08-18-octoterm-desktop-design.md`

## Global Constraints

- 目标平台只有 **Windows 与 macOS**。Linux 不支持,CI 与 release 都不为它构建。
- **不修改 `octoterm-server` 的任何行为**。允许改的只有注释与 README 措辞。`serve` / `router` / `AppState` / `SessionManager` 已经是 `pub`,直接用。
- **`window_size` 在设置界面里只读**,不提供修改。
- 全部注释与用户可见文案用简体中文,与现有代码一致。
- workspace 已有 `edition = "2024"`、`resolver = "2"`,release profile 为 `opt-level='z'` + `lto` + `strip` + `panic="abort"`。新 crate 不覆盖这些。
- 停止 HTTP 层一律用 `JoinHandle::abort()`,**永远不要用 `axum::serve(..).with_graceful_shutdown(..)`** —— WebSocket 是长连接,graceful 会挂死。
- 每个 task 结束时 `cargo clippy --workspace -- -D warnings` 必须干净。

## File Structure

```
crates/desktop/Cargo.toml
crates/desktop/assets/icon.png              托盘图标(模板图:纯黑 + alpha)
crates/desktop/src/lib.rs                   模块声明(与 server crate 同构)
crates/desktop/src/main.rs                  薄入口:单实例 → 日志 → 装配 → 跑事件循环
crates/desktop/src/configfile.rs            toml_edit 就地读写 config.toml
crates/desktop/src/settings/mod.rs
crates/desktop/src/settings/state.rs        表单校验 / 脏标记 / rebind 判定(纯逻辑)
crates/desktop/src/settings/ui.rs           egui 绘制,只读 state
crates/desktop/src/supervisor.rs            内嵌 server 生命周期
crates/desktop/src/logs.rs                  tracing → 日志文件
crates/desktop/src/single_instance.rs       进程锁
crates/desktop/src/autostart.rs             开机自启(mac plist / win 注册表)
crates/desktop/src/tray.rs                  托盘图标与菜单
crates/desktop/src/window.rs                egui + wgpu 窗口的创建/销毁/绘制
crates/desktop/src/app.rs                   winit ApplicationHandler,把上面这些接起来
crates/desktop/tests/configfile.rs
crates/desktop/tests/settings_state.rs
crates/desktop/tests/supervisor.rs
crates/desktop/tests/autostart.rs
scripts/bundle-macos.sh                     组装 .app
scripts/bundle-windows.bat                  收集 exe 产物
```

`lib.rs` + 薄 `main.rs` 是照抄 `crates/server` 的结构 —— 集成测试(`tests/`)只能测 lib,而 `supervisor` 的那条核心测试必须是集成测试。

---

### Task 1: crate 骨架与配置写入

**Files:**
- Create: `crates/desktop/Cargo.toml`
- Create: `crates/desktop/src/lib.rs`
- Create: `crates/desktop/src/main.rs`
- Create: `crates/desktop/src/configfile.rs`
- Create: `crates/desktop/tests/configfile.rs`
- Modify: `Cargo.toml`(workspace members)
- Modify: `.github/workflows/ci.yml`
- Modify: `crates/server/src/config.rs`(`Config::load` 上方注释)
- Modify: `README.md:47`、`README-cnzh.md:40`

**Interfaces:**
- Consumes: 无
- Produces:
  - `octoterm_desktop::configfile::Editable { pub listen: SocketAddr, pub token: Option<String> }`
  - `octoterm_desktop::configfile::default_path() -> anyhow::Result<PathBuf>`
  - `octoterm_desktop::configfile::save(path: &Path, edit: &Editable) -> anyhow::Result<()>`

- [ ] **Step 1: 建 crate 骨架并挂进 workspace**

`crates/desktop/Cargo.toml`:

```toml
[package]
name = "octoterm-desktop"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[lib]
name = "octoterm_desktop"
path = "src/lib.rs"

[[bin]]
name = "octoterm-desktop"
path = "src/main.rs"

[dependencies]
anyhow = "1"
directories = "5"
toml_edit = "0.25"

[dev-dependencies]
tempfile = "3"
```

`crates/desktop/src/lib.rs`:

```rust
pub mod configfile;
```

`crates/desktop/src/main.rs`:

```rust
fn main() {
    println!("octoterm-desktop");
}
```

根 `Cargo.toml` 的 members 改成:

```toml
members = ["crates/protocol", "crates/server", "crates/client-core", "crates/desktop"]
```

- [ ] **Step 2: 写失败的测试**

`crates/desktop/tests/configfile.rs`:

```rust
use octoterm_desktop::configfile::{save, Editable};

fn edit(listen: &str, token: Option<&str>) -> Editable {
    Editable { listen: listen.parse().unwrap(), token: token.map(String::from) }
}

#[test]
fn preserves_comments_and_launcher_sections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"# 我手写的注释,不许动
listen = "127.0.0.1:7683"

# 启动项
[[launcher]]
name = "prod ssh"
command = ["ssh", "prod01"]
"#,
    )
    .unwrap();

    save(&path, &edit("127.0.0.1:9000", None)).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains("# 我手写的注释,不许动"), "注释被碾掉了:\n{out}");
    assert!(out.contains("# 启动项"), "段注释被碾掉了:\n{out}");
    assert!(out.contains(r#"name = "prod ssh""#), "launcher 段丢了:\n{out}");
    assert!(out.contains(r#"listen = "127.0.0.1:9000""#), "listen 没改:\n{out}");
}

#[test]
fn creates_a_minimal_file_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("config.toml");

    save(&path, &edit("0.0.0.0:7683", Some("fixed"))).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains(r#"listen = "0.0.0.0:7683""#), "{out}");
    assert!(out.contains(r#"token = "fixed""#), "{out}");
}

#[test]
fn clearing_the_token_removes_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "listen = \"127.0.0.1:7683\"\ntoken = \"old\"\n").unwrap();

    save(&path, &edit("127.0.0.1:7683", None)).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(!out.contains("token"), "token 键应当被移除:\n{out}");
}

#[test]
fn a_broken_file_is_reported_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let broken = "listen = = =\n";
    std::fs::write(&path, broken).unwrap();

    assert!(save(&path, &edit("127.0.0.1:9000", None)).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken, "坏文件不该被改写");
}

#[test]
fn no_temp_file_is_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    save(&path, &edit("127.0.0.1:7683", None)).unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件:{leftovers:?}");
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p octoterm-desktop --test configfile`
Expected: FAIL,编译错误 `unresolved import octoterm_desktop::configfile`

- [ ] **Step 4: 实现 configfile.rs**

`crates/desktop/src/configfile.rs`:

```rust
//! config.toml 的写入侧。
//!
//! server 自己永远不写这个文件(见 `octoterm_server::config::Config::load`),
//! 写是 desktop 的职责。用 toml_edit 而不是 serde 序列化整个 Config:配置文件
//! 是鼓励用户手写的,把注释、顺序、空行碾掉换不来任何好处。

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml_edit::{value, DocumentMut};

/// desktop 允许改的字段。其余键(window_size、[[launcher]] 等)原样保留。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editable {
    pub listen: SocketAddr,
    /// `None` 表示不固定 token —— 移除该键,server 下次启动随机生成。
    pub token: Option<String>,
}

/// 与 server 用同一个平台配置目录(`octoterm_server::config` 里的私有 `default_path`
/// 的等价实现),两边必须指向同一个文件。
pub fn default_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "octoterm")
        .context("无法确定配置目录")?;
    Ok(dirs.config_dir().join("config.toml"))
}

/// 就地写回。文件不存在时连同父目录一起创建。
pub fn save(path: &Path, edit: &Editable) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut doc: DocumentMut = existing
        .parse()
        .with_context(|| format!("{} 解析失败", path.display()))?;

    doc["listen"] = value(edit.listen.to_string());
    match &edit.token {
        Some(t) => doc["token"] = value(t.as_str()),
        None => {
            doc.remove("token");
        }
    }

    write_atomic(path, doc.to_string().as_bytes())
}

/// 先写同目录的 .tmp 再 rename:写到一半失败不会留下半个配置文件。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("无法创建目录 {}", dir.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("无法写入 {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("无法替换 {}", path.display()))?;
    Ok(())
}
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p octoterm-desktop --test configfile`
Expected: PASS,5 passed

- [ ] **Step 6: 改 CI,把 desktop 排除出 Linux 那格**

`.github/workflows/ci.yml` 里 `Rust tests` 与 `Clippy` 两步改成:

```yaml
      - name: Rust tests
        shell: bash
        run: |
          # desktop 只支持 Windows/macOS,Linux 上要装一堆 X11/Wayland dev 包才能编,不值得
          if [ "${{ runner.os }}" = "Linux" ]; then
            cargo test --workspace --exclude octoterm-desktop
          else
            cargo test --workspace
          fi
      - name: Clippy
        shell: bash
        run: |
          if [ "${{ runner.os }}" = "Linux" ]; then
            cargo clippy --workspace --exclude octoterm-desktop -- -D warnings
          else
            cargo clippy --workspace -- -D warnings
          fi
```

- [ ] **Step 7: 更新两处措辞**

`crates/server/src/config.rs` 里 `Config::load` 上方那行文档注释:

```rust
    /// 只读加载,server 自己永不写文件(写是 octoterm-desktop 的职责):
    /// 显式路径必须存在;缺省路径存在则读,不存在用默认值。
```

`README.md:47` 附近,把 `is never auto-generated; create it yourself at` 改成:

```
is never auto-generated by the server; create it yourself at
```

`README-cnzh.md:40` 附近,把 `配置文件不会自动生成;需要时自行创建` 改成:

```
配置文件不会由 server 自动生成;需要时自行创建
```

- [ ] **Step 8: 全量验证并提交**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings`
Expected: 全绿

```bash
git add crates/desktop Cargo.toml .github/workflows/ci.yml crates/server/src/config.rs README.md README-cnzh.md
git commit -m "feat(desktop): crate 骨架与保留注释的 config.toml 写入"
```

---

### Task 2: 设置表单的校验与 rebind 判定

**Files:**
- Create: `crates/desktop/src/settings/mod.rs`
- Create: `crates/desktop/src/settings/state.rs`
- Create: `crates/desktop/tests/settings_state.rs`
- Modify: `crates/desktop/src/lib.rs`

**Interfaces:**
- Consumes: `configfile::Editable`
- Produces:
  - `settings::state::Form { pub host: String, pub port: String, pub token: String, pub autostart: bool }`
  - `Form::from_current(listen: SocketAddr, autostart: bool) -> Form`
  - `settings::state::FieldError { Host(String), Port(String) }`
  - `Form::validate(&self) -> Result<Editable, FieldError>`
  - `settings::state::needs_rebind(current_listen: SocketAddr, current_token: &str, next: &Editable) -> bool`

- [ ] **Step 1: 写失败的测试**

`crates/desktop/tests/settings_state.rs`:

```rust
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
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p octoterm-desktop --test settings_state`
Expected: FAIL,`unresolved import octoterm_desktop::settings`

- [ ] **Step 3: 实现**

`crates/desktop/src/settings/mod.rs`:

```rust
pub mod state;
```

`crates/desktop/src/settings/state.rs`:

```rust
//! 设置窗口的状态与校验。这里不知道 egui 存在 —— 全部逻辑都能直接跑单测。

use std::net::{IpAddr, SocketAddr};

use crate::configfile::Editable;

/// 表单里存字符串而不是强类型:用户打字的过程中,绝大多数时刻内容都是非法的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Form {
    pub host: String,
    pub port: String,
    /// 空白表示「不固定」。
    pub token: String,
    pub autostart: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldError {
    Host(String),
    Port(String),
}

impl std::fmt::Display for FieldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldError::Host(m) | FieldError::Port(m) => f.write_str(m),
        }
    }
}

impl Form {
    pub fn from_current(listen: SocketAddr, autostart: bool) -> Self {
        Self {
            host: listen.ip().to_string(),
            port: listen.port().to_string(),
            // token 不从当前值回填:它是密钥,默认展示为空(= 不固定)容易误伤,
            // 所以由调用方在构造后显式赋值,见 settings/ui.rs。
            token: String::new(),
            autostart,
        }
    }

    pub fn validate(&self) -> Result<Editable, FieldError> {
        let ip: IpAddr = self
            .host
            .trim()
            .parse()
            .map_err(|_| FieldError::Host(format!("不是合法的 IP 地址:{}", self.host.trim())))?;
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| FieldError::Port(format!("端口必须是 1-65535 的整数:{}", self.port.trim())))?;
        if port == 0 {
            // 0 会让系统随机分配端口,对一个要被访问的服务没有意义
            return Err(FieldError::Port("端口不能是 0".into()));
        }
        let token = self.token.trim();
        Ok(Editable {
            listen: SocketAddr::new(ip, port),
            token: (!token.is_empty()).then(|| token.to_string()),
        })
    }
}

/// 保存时要不要重启 HTTP 层。
///
/// 注意 `token: None`(不固定)**不**触发 rebind:它只是把键从 config.toml 拿掉,
/// 本次运行继续用现有 token,下次启动才随机 —— 否则保存一下就把自己踢下线了。
pub fn needs_rebind(current_listen: SocketAddr, current_token: &str, next: &Editable) -> bool {
    next.listen != current_listen
        || next.token.as_deref().is_some_and(|t| t != current_token)
}
```

`crates/desktop/src/lib.rs` 加一行:

```rust
pub mod configfile;
pub mod settings;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p octoterm-desktop --test settings_state`
Expected: PASS,8 passed

- [ ] **Step 5: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 设置表单的校验与 rebind 判定"
```

---

### Task 3: Supervisor —— 改配置不丢会话

这是整个 crate 的核心。Task 结束时那条测试直接把设计承诺钉死。

**Files:**
- Create: `crates/desktop/src/supervisor.rs`
- Create: `crates/desktop/tests/supervisor.rs`
- Modify: `crates/desktop/src/lib.rs`、`crates/desktop/Cargo.toml`

**Interfaces:**
- Consumes: `octoterm_server::{app::{serve, AppState}, session::manager::SessionManager, launcher, config::WindowSize}`
- Produces:
  - `supervisor::Supervisor::new(buffer_cap: usize, window_size: WindowSize, specs: &[LauncherSpec]) -> Supervisor`
  - `Supervisor::manager(&self) -> &Arc<SessionManager>`
  - `Supervisor::listen(&self) -> Option<SocketAddr>`
  - `Supervisor::token(&self) -> Option<&str>`
  - `Supervisor::restart(&mut self, listen: SocketAddr, token: String) -> impl Future<Output = anyhow::Result<SocketAddr>>`
  - `Supervisor::stop(&mut self)`

- [ ] **Step 1: 加依赖**

`crates/desktop/Cargo.toml` 的 `[dependencies]` 追加:

```toml
octoterm-server = { path = "../server" }
tokio = { version = "1", features = ["rt-multi-thread", "net", "time", "macros", "sync"] }
tracing = "0.1"
```

`[dev-dependencies]` 追加:

```toml
tokio = { version = "1", features = ["full", "test-util"] }
```

- [ ] **Step 2: 写失败的测试**

`crates/desktop/tests/supervisor.rs`:

```rust
use std::time::Duration;

use octoterm_desktop::supervisor::Supervisor;
use octoterm_server::config::WindowSize;

/// 一个不会自己退出的会话,测完由 kill 收尾。
fn long_lived_cmd() -> Option<Vec<String>> {
    #[cfg(unix)]
    return Some(vec!["/bin/sh".into(), "-i".into()]);
    #[cfg(windows)]
    return None; // 默认 powershell
}

/// abort 之后端口的释放是异步的,给它一点时间。
async fn wait_until_refused(addr: std::net::SocketAddr) -> bool {
    for _ in 0..50 {
        if tokio::net::TcpStream::connect(addr).await.is_err() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn rebinding_to_a_new_port_keeps_sessions_alive() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let old = sup.restart("127.0.0.1:0".parse().unwrap(), "t1".into()).await.unwrap();

    sup.manager().create(None, long_lived_cmd(), None).unwrap();
    assert_eq!(sup.manager().list().len(), 1);

    let new = sup.restart("127.0.0.1:0".parse().unwrap(), "t1".into()).await.unwrap();
    assert_ne!(old, new, "端口 0 每次应当分到不同端口");

    assert!(wait_until_refused(old).await, "旧端口仍在接受连接");
    assert!(tokio::net::TcpStream::connect(new).await.is_ok(), "新端口不可连");
    assert_eq!(sup.manager().list().len(), 1, "rebind 不该丢会话");

    let id = sup.manager().list()[0].id;
    sup.manager().kill(id);
}

#[tokio::test]
async fn changing_only_the_token_reuses_the_same_address() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "old".into()).await.unwrap();

    // 同地址重启:先关后 bind + 重试,必须仍然成功且地址不变
    let again = sup.restart(addr, "new".into()).await.unwrap();
    assert_eq!(addr, again);
    assert_eq!(sup.token(), Some("new"));
    assert!(tokio::net::TcpStream::connect(again).await.is_ok());
}

#[tokio::test]
async fn a_failed_bind_leaves_the_old_listener_running() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();

    // 占住另一个端口,再让 supervisor 去抢它
    let squatter = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let taken = squatter.local_addr().unwrap();

    assert!(sup.restart(taken, "t".into()).await.is_err(), "抢占用端口应当失败");
    assert_eq!(sup.listen(), Some(addr), "失败后应当仍在原地址上");
    assert!(tokio::net::TcpStream::connect(addr).await.is_ok(), "旧 listener 被误关了");
}

#[tokio::test]
async fn stop_releases_the_port_but_not_the_sessions() {
    let mut sup = Supervisor::new(1 << 20, WindowSize::default(), &[]);
    let addr = sup.restart("127.0.0.1:0".parse().unwrap(), "t".into()).await.unwrap();
    sup.manager().create(None, long_lived_cmd(), None).unwrap();

    sup.stop();

    assert_eq!(sup.listen(), None);
    assert!(wait_until_refused(addr).await);
    assert_eq!(sup.manager().list().len(), 1, "停 HTTP 层不该动会话");

    let id = sup.manager().list()[0].id;
    sup.manager().kill(id);
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p octoterm-desktop --test supervisor`
Expected: FAIL,`unresolved import octoterm_desktop::supervisor`

- [ ] **Step 4: 实现**

`crates/desktop/src/supervisor.rs`:

```rust
//! 内嵌 server 的生命周期。
//!
//! 全部要点只有一句:`SessionManager` 由 Supervisor **长期持有**,跨 restart 不
//! 重建。HTTP 层(listener + AppState)可以随便拆了重搭,pty 会话一个都不会丢 ——
//! 客户端看到的只是一次断线,按既有的 seamless resume 自己接回来。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use octoterm_server::app::{serve, AppState};
use octoterm_server::config::{LauncherSpec, WindowSize};
use octoterm_server::launcher::LauncherProvider;
use octoterm_server::session::manager::SessionManager;
use tokio::net::TcpListener;

struct Running {
    listen: SocketAddr,
    token: String,
    join: tokio::task::JoinHandle<()>,
}

pub struct Supervisor {
    manager: Arc<SessionManager>,
    launchers: Arc<Vec<Box<dyn LauncherProvider>>>,
    running: Option<Running>,
}

impl Supervisor {
    pub fn new(buffer_cap: usize, window_size: WindowSize, specs: &[LauncherSpec]) -> Self {
        Self {
            manager: SessionManager::new(buffer_cap, window_size),
            launchers: Arc::new(octoterm_server::launcher::providers(specs)),
            running: None,
        }
    }

    pub fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }

    pub fn listen(&self) -> Option<SocketAddr> {
        self.running.as_ref().map(|r| r.listen)
    }

    pub fn token(&self) -> Option<&str> {
        self.running.as_ref().map(|r| r.token.as_str())
    }

    /// 用新的监听地址与 token 重建 HTTP 层。返回实际监听到的地址(端口 0 会被
    /// 系统换成真实端口)。
    ///
    /// 地址**有变化**时先 bind 新的、成功了才关旧的 —— 端口被占用这种最常见的
    /// 失败不会把用户锁在外面。地址**没变**时做不到先 bind(同一地址上不能有两个
    /// listener,`SO_REUSEPORT` 在 Windows 上不可用),只能先关再带重试地 bind。
    pub async fn restart(&mut self, listen: SocketAddr, token: String) -> Result<SocketAddr> {
        let same_addr = self.running.as_ref().is_some_and(|r| r.listen == listen);
        let listener = if same_addr {
            self.stop();
            bind_with_retry(listen).await?
        } else {
            let l = TcpListener::bind(listen)
                .await
                .with_context(|| format!("无法监听 {listen}"))?;
            self.stop();
            l
        };
        let actual = listener.local_addr()?;
        let state = AppState {
            manager: self.manager.clone(),
            token: token.clone(),
            launchers: self.launchers.clone(),
        };
        let join = tokio::spawn(async move {
            if let Err(e) = serve(listener, state).await {
                tracing::error!(error = %e, "http 层异常退出");
            }
        });
        tracing::info!(%actual, "http 层已就绪");
        self.running = Some(Running { listen: actual, token, join });
        Ok(actual)
    }

    /// 停掉 HTTP 层。会话与 `SessionManager` 不受影响。
    ///
    /// 用 abort 而不是 axum 的 graceful shutdown:graceful 会等所有连接结束,而
    /// WebSocket 是长连接,永远不结束 —— 那等于挂死。
    pub fn stop(&mut self) {
        if let Some(r) = self.running.take() {
            r.join.abort();
            tracing::info!(listen = %r.listen, "http 层已停止");
        }
    }
}

/// 端口刚被自己 abort 释放,操作系统那边是异步的,给几次机会。
async fn bind_with_retry(listen: SocketAddr) -> Result<TcpListener> {
    let mut last = None;
    for _ in 0..10 {
        match TcpListener::bind(listen).await {
            Ok(l) => return Ok(l),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    Err(last.unwrap()).with_context(|| format!("无法监听 {listen}"))
}
```

`crates/desktop/src/lib.rs`:

```rust
pub mod configfile;
pub mod settings;
pub mod supervisor;
```

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p octoterm-desktop --test supervisor`
Expected: PASS,4 passed

如果 `changing_only_the_token_reuses_the_same_address` 偶发失败,说明 10 次 × 20ms 的重试窗口不够 —— 提高到 20 次,不要改成"忽略错误"。

- [ ] **Step 6: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): Supervisor —— 重建 HTTP 层而不丢 pty 会话"
```

---

### Task 4: 日志与单实例

**Files:**
- Create: `crates/desktop/src/logs.rs`
- Create: `crates/desktop/src/single_instance.rs`
- Modify: `crates/desktop/src/lib.rs`、`crates/desktop/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces:
  - `logs::log_path() -> anyhow::Result<PathBuf>`
  - `logs::init() -> anyhow::Result<PathBuf>`(装 subscriber,返回日志路径)
  - `logs::truncate_if_larger_than(path: &Path, limit: u64) -> std::io::Result<()>`
  - `single_instance::Guard`(RAII,drop 即释放)
  - `single_instance::acquire(path: &Path) -> anyhow::Result<Option<Guard>>`(`Ok(None)` = 已有实例)

- [ ] **Step 1: 加依赖**

`crates/desktop/Cargo.toml` 的 `[dependencies]` 追加:

```toml
fs4 = { version = "1", features = ["sync"] }
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
```

- [ ] **Step 2: 写失败的测试**

在 `crates/desktop/src/single_instance.rs` 末尾(实现之后)与 `logs.rs` 末尾各放一个 `#[cfg(test)] mod tests` —— 这两个模块的测试不需要跨 crate 边界,放内联更近。先把测试写出来:

`crates/desktop/src/single_instance.rs` 的测试部分:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_acquire_on_the_same_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.lock");

        let first = acquire(&path).unwrap();
        assert!(first.is_some(), "第一个实例应当拿到锁");

        let second = acquire(&path).unwrap();
        assert!(second.is_none(), "第二个实例应当被拒绝");
    }

    #[test]
    fn releasing_the_guard_lets_the_next_one_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.lock");

        drop(acquire(&path).unwrap());
        assert!(acquire(&path).unwrap().is_some(), "锁没被释放");
    }
}
```

`crates/desktop/src/logs.rs` 的测试部分:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_oversized_log_is_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.log");
        std::fs::write(&path, vec![b'x'; 4096]).unwrap();

        truncate_if_larger_than(&path, 1024).unwrap();

        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
    }

    #[test]
    fn a_small_log_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("octoterm.log");
        std::fs::write(&path, b"hello").unwrap();

        truncate_if_larger_than(&path, 1024).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[test]
    fn a_missing_log_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        truncate_if_larger_than(&dir.path().join("nope.log"), 1024).unwrap();
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p octoterm-desktop --lib`
Expected: FAIL,`cannot find function acquire` / `truncate_if_larger_than`

- [ ] **Step 4: 实现 single_instance.rs**

```rust
//! 进程锁。两份 desktop 同时跑必然抢同一个端口,直接在启动时挡掉。
//!
//! 用文件锁而不是端口探测:端口可能被别的程序占着,那不代表已有 desktop 实例。
//! 操作系统在进程死亡时自动释放文件锁,所以崩溃不会留下僵尸锁。

use std::fs::{File, OpenOptions};
use std::path::Path;

use anyhow::{Context, Result};
use fs4::fs_std::FileExt;

/// 持有它就代表持有锁;drop 即释放。
pub struct Guard(File);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

/// `Ok(None)` 表示已经有另一个实例在跑(不是错误,是正常分支)。
pub fn acquire(path: &Path) -> Result<Option<Guard>> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("无法打开锁文件 {}", path.display()))?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(true) => Ok(Some(Guard(file))),
        Ok(false) => Ok(None),
        Err(e) => Err(e).context("锁文件失败"),
    }
}
```

> `fs4` 的 trait 路径与 `try_lock_exclusive` 的返回类型请以 `cargo doc -p fs4 --open` 为准
> (1.x 里是 `fs4::fs_std::FileExt`,返回 `io::Result<bool>`)。若签名不同,只调整这
> 三行,`acquire` 的对外语义(`Ok(None)` = 已有实例)不变。

- [ ] **Step 5: 实现 logs.rs**

```rust
//! GUI 进程没有可见的 stderr(macOS 是双击 .app,Windows 是 windows subsystem),
//! 所以日志必须落盘,托盘菜单用系统默认程序打开它。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 超过这个大小就在启动时清空。不做滚动归档 —— 一个随手用的工具不需要日志考古。
const MAX_LOG_BYTES: u64 = 1 << 20;

pub fn log_path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "octoterm")
        .context("无法确定配置目录")?;
    Ok(dirs.config_dir().join("octoterm.log"))
}

pub fn truncate_if_larger_than(path: &Path, limit: u64) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > limit => std::fs::write(path, b""),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// 装上全局 subscriber,返回日志文件路径。
pub fn init() -> Result<PathBuf> {
    let path = log_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    truncate_if_larger_than(&path, MAX_LOG_BYTES)?;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("无法打开日志 {}", path.display()))?;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(file))
        .init();
    Ok(path)
}
```

`crates/desktop/src/lib.rs`:

```rust
pub mod configfile;
pub mod logs;
pub mod settings;
pub mod single_instance;
pub mod supervisor;
```

- [ ] **Step 6: 跑测试确认通过**

Run: `cargo test -p octoterm-desktop --lib`
Expected: PASS,5 passed

- [ ] **Step 7: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 日志落盘与单实例锁"
```

---

### Task 5: 开机自启

**Files:**
- Create: `crates/desktop/src/autostart.rs`
- Create: `crates/desktop/tests/autostart.rs`
- Modify: `crates/desktop/src/lib.rs`、`crates/desktop/Cargo.toml`

**Interfaces:**
- Consumes: 无
- Produces:
  - `autostart::is_enabled() -> anyhow::Result<bool>`
  - `autostart::set(enabled: bool) -> anyhow::Result<()>`
  - `autostart::plist_xml(exe: &str) -> String`(macOS,纯函数,可测)
  - `autostart::LABEL: &str = "com.octoterm.desktop"`

- [ ] **Step 1: 加依赖**

`crates/desktop/Cargo.toml` 追加:

```toml
[target.'cfg(windows)'.dependencies]
winreg = "0.56"
```

- [ ] **Step 2: 写失败的测试**

`crates/desktop/tests/autostart.rs`:

```rust
#[cfg(target_os = "macos")]
#[test]
fn the_plist_names_the_executable_and_runs_at_load() {
    let xml = octoterm_desktop::autostart::plist_xml("/Applications/octoterm.app/Contents/MacOS/octoterm-desktop");

    assert!(xml.contains("<key>Label</key>"));
    assert!(xml.contains("com.octoterm.desktop"));
    assert!(xml.contains("/Applications/octoterm.app/Contents/MacOS/octoterm-desktop"));
    assert!(xml.contains("<key>RunAtLoad</key>"));
    assert!(xml.contains("<true/>"));
    // LaunchAgent 只该在登录时拉起一次,不该被当成需要常活的守护
    assert!(!xml.contains("KeepAlive"));
}

#[cfg(target_os = "macos")]
#[test]
fn a_path_with_an_ampersand_is_escaped() {
    let xml = octoterm_desktop::autostart::plist_xml("/Users/a&b/octoterm-desktop");
    assert!(xml.contains("/Users/a&amp;b/octoterm-desktop"), "路径没做 XML 转义:\n{xml}");
    assert!(!xml.contains("a&b"));
}

// 不写「启用→读回→禁用」的往返测试:它会真的改动当前用户的登录项
// (~/Library/LaunchAgents 或 HKCU Run),测试中途 panic 就会把 octoterm 留在
// 开机启动里。`is_enabled` / `set` 的真实读写由 Task 8 的手动验收覆盖。
```

- [ ] **Step 3: 跑测试确认失败**

Run: `cargo test -p octoterm-desktop --test autostart`
Expected: FAIL,`unresolved import octoterm_desktop::autostart`

- [ ] **Step 4: 实现**

`crates/desktop/src/autostart.rs`:

```rust
//! 开机自启。这一项不进 config.toml —— 它是 desktop 自己的行为,不是 server 的配置。
//!
//! macOS 用 LaunchAgent plist(放进 ~/Library/LaunchAgents 就会在下次登录时生效,
//! 不需要 launchctl),Windows 用 HKCU 的 Run 键。

use anyhow::{Context, Result};

pub const LABEL: &str = "com.octoterm.desktop";

fn exe_path() -> Result<String> {
    Ok(std::env::current_exe()
        .context("无法取得自身路径")?
        .to_string_lossy()
        .into_owned())
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::path::PathBuf;

    fn xml_escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    pub fn plist_xml(exe: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
            label = LABEL,
            exe = xml_escape(exe),
        )
    }

    fn plist_path() -> Result<PathBuf> {
        let home = directories::BaseDirs::new().context("无法确定 HOME")?;
        Ok(home.home_dir().join("Library/LaunchAgents").join(format!("{LABEL}.plist")))
    }

    pub fn is_enabled() -> Result<bool> {
        Ok(plist_path()?.exists())
    }

    pub fn set(enabled: bool) -> Result<()> {
        let path = plist_path()?;
        if enabled {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&path, plist_xml(&exe_path()?))
                .with_context(|| format!("无法写入 {}", path.display()))?;
        } else if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("无法删除 {}", path.display()))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE: &str = "octoterm";

    pub fn is_enabled() -> Result<bool> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(run) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ) else {
            return Ok(false);
        };
        Ok(run.get_value::<String, _>(VALUE).is_ok())
    }

    pub fn set(enabled: bool) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu
            .create_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)
            .context("无法打开 Run 注册表键")?;
        if enabled {
            // 加引号:路径里有空格时 Windows 才不会把它当成多个参数
            run.set_value(VALUE, &format!("\"{}\"", exe_path()?))
                .context("无法写入自启项")?;
        } else {
            match run.delete_value(VALUE) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).context("无法删除自启项"),
            }
        }
        Ok(())
    }
}

pub use imp::{is_enabled, set};

#[cfg(target_os = "macos")]
pub use imp::plist_xml;
```

`crates/desktop/src/lib.rs` 追加 `pub mod autostart;`。

- [ ] **Step 5: 跑测试确认通过**

Run: `cargo test -p octoterm-desktop --test autostart`
Expected: macOS 上 2 passed;Windows 上无适用测试(纯函数是 macOS 专有),autostart 的真实读写由 Task 8 手动验收覆盖

- [ ] **Step 6: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 开机自启(macOS LaunchAgent / Windows Run 键)"
```

---

### Task 6: 托盘 —— 第一个能跑起来的版本

这一 task 结束时,程序应当能真的跑:托盘图标出现、菜单能点、"打开 Web 客户端"能打开浏览器并连上会话、"退出"能退出。

**Files:**
- Create: `crates/desktop/assets/icon.png`
- Create: `crates/desktop/src/tray.rs`
- Create: `crates/desktop/src/app.rs`
- Modify: `crates/desktop/src/main.rs`、`crates/desktop/src/lib.rs`、`crates/desktop/Cargo.toml`

**Interfaces:**
- Consumes: `supervisor::Supervisor`、`logs`、`single_instance`、`configfile`
- Produces:
  - `app::UserEvent { MenuClicked(MenuAction), SessionsChanged }`
  - `app::MenuAction { OpenWeb, CopyUrl, Settings, ViewLogs, Quit }`
  - `tray::Tray::new(proxy: EventLoopProxy<UserEvent>) -> anyhow::Result<Tray>`
  - `tray::Tray::set_status(&mut self, text: &str)`(状态行 + tooltip)

- [ ] **Step 1: 加依赖并准备图标**

`crates/desktop/Cargo.toml` 追加:

```toml
winit = "0.30.13"
tray-icon = "0.24"
png = "0.18"
arboard = "3"
open = "5"
```

`arboard` 用于「复制访问链接」,`open` 用于「打开 Web 客户端 / 查看日志 / 打开 config.toml」。

图标:准备一张 32×32 的 PNG,**纯黑前景 + alpha 通道**(macOS 模板图的要求;Windows 上照样显示)存为 `crates/desktop/assets/icon.png`。可以用任何工具画;临时占位可以用:

```bash
python3 -c "
import zlib,struct
w=h=32
rows=b''
for y in range(h):
    row=b'\x00'
    for x in range(w):
        inside = 6 <= x < 26 and 6 <= y < 26 and not (9 <= x < 23 and 9 <= y < 23)
        row += bytes((0,0,0,255 if inside else 0))
    rows+=row
def chunk(t,d):
    c=t+d
    return struct.pack('>I',len(d))+c+struct.pack('>I',zlib.crc32(c))
png=b'\x89PNG\r\n\x1a\n'
png+=chunk(b'IHDR',struct.pack('>IIBBBBB',w,h,8,6,0,0,0))
png+=chunk(b'IDAT',zlib.compress(rows))
png+=chunk(b'IEND',b'')
open('crates/desktop/assets/icon.png','wb').write(png)
"
```

- [ ] **Step 2: 实现 tray.rs**

```rust
//! 托盘图标与菜单。
//!
//! 菜单事件不靠轮询:tray-icon / muda 支持注册全局 handler,我们在 handler 里
//! 把事件转成 winit 的 user event 发进主事件循环,这样所有状态变更都在同一个
//! 线程上串行发生。

use anyhow::{Context, Result};
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::event_loop::EventLoopProxy;

use crate::app::{MenuAction, UserEvent};

pub struct Tray {
    icon: TrayIcon,
    status: MenuItem,
}

fn decode_icon() -> Result<Icon> {
    // 图标嵌进二进制,和 server 用 rust-embed 嵌前端是同一个思路:分发只有一个文件
    let bytes = include_bytes!("../assets/icon.png");
    let decoder = png::Decoder::new(&bytes[..]);
    let mut reader = decoder.read_info().context("托盘图标不是合法 PNG")?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).context("托盘图标解码失败")?;
    anyhow::ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "托盘图标必须是 8 位 RGBA"
    );
    buf.truncate(info.buffer_size());
    Icon::from_rgba(buf, info.width, info.height).context("托盘图标构造失败")
}

impl Tray {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Result<Self> {
        let status = MenuItem::new("octoterm — 启动中…", false, None);
        let open_web = MenuItem::new("打开 Web 客户端", true, None);
        let copy_url = MenuItem::new("复制访问链接", true, None);
        let settings = MenuItem::new("设置…", true, None);
        let view_logs = MenuItem::new("查看日志…", true, None);
        let quit = MenuItem::new("退出", true, None);

        let ids: Vec<(MenuId, MenuAction)> = vec![
            (open_web.id().clone(), MenuAction::OpenWeb),
            (copy_url.id().clone(), MenuAction::CopyUrl),
            (settings.id().clone(), MenuAction::Settings),
            (view_logs.id().clone(), MenuAction::ViewLogs),
            (quit.id().clone(), MenuAction::Quit),
        ];

        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &open_web,
            &copy_url,
            &PredefinedMenuItem::separator(),
            &settings,
            &view_logs,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, action)) = ids.iter().find(|(id, _)| *id == e.id) {
                let _ = proxy.send_event(UserEvent::MenuClicked(*action));
            }
        }));

        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("octoterm")
            .with_icon(decode_icon()?);
        // macOS 的菜单栏会随亮/暗色反转模板图;不设这个,暗色下就是一坨黑块
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_as_template(true);

        Ok(Self { icon: builder.build().context("无法创建托盘图标")?, status })
    }

    /// 状态行 + tooltip 用同一段文字。tooltip 只是设个字符串,比每次会话变化都
    /// 重建整个菜单便宜得多。
    pub fn set_status(&mut self, text: &str) {
        self.status.set_text(text);
        let _ = self.icon.set_tooltip(Some(text));
    }
}
```

> `with_icon_as_template` 是 macOS 专有扩展,在 tray-icon 0.24 里可能需要
> `use tray_icon::TrayIconBuilderExtMacOS;`。跑 `cargo build` 按编译器提示补 use。

- [ ] **Step 3: 实现 app.rs**

```rust
//! winit 的 ApplicationHandler:所有状态都在主线程上,tokio 在后台线程。

use std::sync::Arc;

use anyhow::Result;
use octoterm_server::session::manager::SessionManager;
use tokio::runtime::Runtime;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::WindowId;

use crate::supervisor::Supervisor;
use crate::tray::Tray;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    OpenWeb,
    CopyUrl,
    Settings,
    ViewLogs,
    Quit,
}

#[derive(Debug, Clone)]
pub enum UserEvent {
    MenuClicked(MenuAction),
    SessionsChanged,
}

pub struct App {
    rt: Runtime,
    sup: Supervisor,
    tray: Option<Tray>,
    proxy: EventLoopProxy<UserEvent>,
    log_path: std::path::PathBuf,
}

impl App {
    pub fn new(
        rt: Runtime,
        sup: Supervisor,
        proxy: EventLoopProxy<UserEvent>,
        log_path: std::path::PathBuf,
    ) -> Self {
        Self { rt, sup, tray: None, proxy, log_path }
    }

    /// 带 token 的访问 URL,和 CLI 启动时打印的那一行是同一格式。
    fn url(&self) -> Option<String> {
        let listen = self.sup.listen()?;
        let token = self.sup.token()?;
        let ip = listen.ip();
        let host = if ip.is_unspecified() {
            "127.0.0.1".to_string()
        } else if ip.is_ipv6() {
            format!("[{ip}]")
        } else {
            ip.to_string()
        };
        Some(format!("http://{host}:{}/#token={token}", listen.port()))
    }

    fn status_text(&self) -> String {
        match self.sup.listen() {
            Some(addr) => {
                let n = self.sup.manager().list().len();
                format!("octoterm · {addr} · {n} 个会话")
            }
            None => "octoterm · 未监听".to_string(),
        }
    }

    fn refresh_status(&mut self) {
        let text = self.status_text();
        if let Some(tray) = self.tray.as_mut() {
            tray.set_status(&text);
        }
    }

    /// 订阅会话事件,变化时叫主线程刷新状态行。
    fn watch_sessions(&self, manager: Arc<SessionManager>) {
        let proxy = self.proxy.clone();
        self.rt.spawn(async move {
            let mut rx = manager.events();
            while rx.recv().await.is_ok() {
                if proxy.send_event(UserEvent::SessionsChanged).is_err() {
                    break; // 事件循环没了,收工
                }
            }
        });
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // 托盘常驻应用启动时不建窗口:设置窗口在用户点「设置…」时才创建。
        if self.tray.is_none() {
            match Tray::new(self.proxy.clone()) {
                Ok(tray) => self.tray = Some(tray),
                Err(e) => {
                    tracing::error!(error = %e, "托盘创建失败");
                    return;
                }
            }
            self.watch_sessions(self.sup.manager().clone());
            self.refresh_status();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::SessionsChanged => self.refresh_status(),
            UserEvent::MenuClicked(MenuAction::OpenWeb) => {
                if let Some(url) = self.url() {
                    let _ = open::that_detached(url);
                }
            }
            UserEvent::MenuClicked(MenuAction::CopyUrl) => {
                if let Some(url) = self.url() {
                    match arboard::Clipboard::new().and_then(|mut c| c.set_text(url)) {
                        Ok(()) => {}
                        Err(e) => tracing::error!(error = %e, "复制到剪贴板失败"),
                    }
                }
            }
            UserEvent::MenuClicked(MenuAction::ViewLogs) => {
                let _ = open::that_detached(&self.log_path);
            }
            UserEvent::MenuClicked(MenuAction::Settings) => {
                tracing::info!("设置窗口尚未实现"); // Task 8 接上
            }
            UserEvent::MenuClicked(MenuAction::Quit) => {
                self.sup.stop();
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {
        // Task 7 起有窗口了再处理
    }
}
```

- [ ] **Step 4: 实现 main.rs**

```rust
// GUI 进程不要控制台窗口
#![cfg_attr(windows, windows_subsystem = "windows")]

use anyhow::{Context, Result};
use octoterm_desktop::app::{App, UserEvent};
use octoterm_desktop::supervisor::Supervisor;
use octoterm_desktop::{configfile, logs, single_instance};
use octoterm_server::config::Config;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    let log_path = logs::init()?;

    let lock_path = configfile::default_path()?.with_file_name("octoterm.lock");
    let _guard = match single_instance::acquire(&lock_path)? {
        Some(g) => g,
        None => {
            tracing::warn!("已有 octoterm-desktop 实例在运行,退出");
            return Ok(());
        }
    };

    // 配置读不出来不是致命错误:托盘照样要出来,用户才有地方修它(见 Task 9)
    let config = Config::load(None).unwrap_or_default();
    let (token, _) = octoterm_server::config::resolve_token(None, config.token.clone());

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("无法创建 tokio runtime")?;

    let mut sup = Supervisor::new(1 << 20, config.window_size, &config.launchers);
    if let Err(e) = rt.block_on(sup.restart(config.listen, token)) {
        tracing::error!(error = %e, "启动时监听失败");
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("无法创建事件循环")?;
    // 托盘常驻应用不占 Dock、不接管菜单栏
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        let _ = ActivationPolicy::Accessory; // 见下方 Step 5 的说明
    }
    let proxy = event_loop.create_proxy();
    let mut app = App::new(rt, sup, proxy, log_path);
    event_loop.run_app(&mut app).context("事件循环异常退出")?;
    Ok(())
}
```

`crates/desktop/src/lib.rs`:

```rust
pub mod app;
pub mod autostart;
pub mod configfile;
pub mod logs;
pub mod settings;
pub mod single_instance;
pub mod supervisor;
pub mod tray;
```

- [ ] **Step 5: 修正 macOS activation policy 的设置位置**

上一步留了个占位:`ActivationPolicy` 必须在 **构建** event loop 时通过
`EventLoop::with_user_event().with_activation_policy(ActivationPolicy::Accessory)`
设置(`EventLoopBuilderExtMacOS` 提供的方法),而不是构建之后。把 main.rs 里那段改成:

```rust
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        builder.with_activation_policy(ActivationPolicy::Accessory);
    }
    let event_loop = builder.build().context("无法创建事件循环")?;
```

跑 `cargo build -p octoterm-desktop` 让编译器确认方法名与接收者形式(0.30 里是
`&mut self` 链式),按提示微调。

- [ ] **Step 6: 编译并手动验收**

Run: `cargo build -p octoterm-desktop && cargo run -p octoterm-desktop`

手动检查清单:
1. 托盘出现图标(macOS 菜单栏 / Windows 通知区域)
2. macOS:**Dock 里没有图标**,也没有接管顶部菜单栏
3. 点菜单,状态行显示 `octoterm · 127.0.0.1:7683 · 0 个会话`
4. 点「打开 Web 客户端」,浏览器打开并能新建会话
5. 新建会话后再点开菜单,状态行的会话数变成 1
6. 点「复制访问链接」,粘贴出来是带 token 的 URL
7. 点「查看日志」,系统默认程序打开 `octoterm.log`
8. 点「退出」,进程结束,托盘图标消失
9. 再启动两次,第二个实例应当立刻退出(检查日志里的「已有 octoterm-desktop 实例」)

- [ ] **Step 7: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 托盘图标与菜单,内嵌 server 可跑通"
```

---

### Task 7: 按需创建/销毁的 egui 窗口

**Files:**
- Create: `crates/desktop/src/window.rs`
- Modify: `crates/desktop/src/app.rs`、`crates/desktop/src/lib.rs`、`crates/desktop/Cargo.toml`

**Interfaces:**
- Consumes: `app::UserEvent`
- Produces:
  - `window::EguiWindow::open(event_loop: &ActiveEventLoop, title: &str, size: (u32, u32)) -> anyhow::Result<EguiWindow>`
  - `EguiWindow::id(&self) -> WindowId`
  - `EguiWindow::on_window_event(&mut self, event: &WindowEvent) -> bool`(返回 true 表示 egui 消费了它)
  - `EguiWindow::redraw(&mut self, ui: impl FnOnce(&egui::Context))`
  - `EguiWindow::request_redraw(&self)`

`EguiWindow` 的 `Drop` 必须释放 wgpu 的 surface 与 device —— 关窗口就是真的关。

- [ ] **Step 1: 加依赖**

```toml
egui = "0.36"
egui-winit = { version = "0.36", default-features = false }
egui-wgpu = { version = "0.36", features = ["winit"] }
wgpu = "30"
pollster = "0.4"
```

`egui-winit` 关掉默认特性是为了不把 `accesskit`、X11/Wayland 那些拉进来。若关掉后
编译报缺特性,按编译器提示逐个加回,**不要**直接开 `default-features = true`。

- [ ] **Step 2: 实现 window.rs**

这是本计划里唯一一块"按文档现编"的代码 —— egui-wgpu 0.36 的 `Painter` 签名请以
`cargo doc -p egui-wgpu --open` 为准。**对外接口(上面 Interfaces 那几个方法)是固定的**,
内部怎么写以编译通过为准。骨架:

```rust
//! 设置窗口的载体:一个按需创建、关掉就真的销毁的 egui 窗口。
//!
//! 不用 eframe:它假定自己拥有事件循环、并在启动时就建窗口,而托盘常驻应用要的
//! 恰恰是「平时 0 窗口」。多出来的这点管线代码换的是空闲时进程里只剩 tokio 和一
//! 个状态栏图标。
//!
//! 渲染后端是 wgpu 而不是 glow:OpenGL 在 macOS 自 10.14 起已废弃,wgpu 走
//! Metal / DX12,是受支持的原生路径。

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

pub struct EguiWindow {
    window: Arc<Window>,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    painter: egui_wgpu::winit::Painter,
}

impl EguiWindow {
    pub fn open(event_loop: &ActiveEventLoop, title: &str, size: (u32, u32)) -> Result<Self> {
        let attrs = Window::default_attributes()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1))
            .with_resizable(true);
        let window = Arc::new(event_loop.create_window(attrs).context("无法创建窗口")?);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let mut painter = egui_wgpu::winit::Painter::new(
            egui_ctx.clone(),
            egui_wgpu::WgpuConfiguration::default(),
            1,     // msaa samples
            None,  // depth format
            false, // dithering
        );
        pollster::block_on(painter.set_window(egui::ViewportId::ROOT, Some(window.clone())))
            .context("无法初始化 wgpu 渲染器")?;

        Ok(Self { window, egui_ctx, egui_state, painter })
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    /// 返回 true 表示事件被 egui 吃掉了(比如点在文本框里)。
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        self.egui_state.on_window_event(&self.window, event).consumed
    }

    pub fn redraw(&mut self, ui: impl FnOnce(&egui::Context)) {
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let output = self.egui_ctx.run(raw_input, |ctx| ui(ctx));
        self.egui_state
            .handle_platform_output(&self.window, output.platform_output.clone());
        let primitives = self.egui_ctx.tessellate(output.shapes, output.pixels_per_point);
        self.painter.paint_and_update_textures(
            egui::ViewportId::ROOT,
            output.pixels_per_point,
            [0.0, 0.0, 0.0, 0.0],
            &primitives,
            &output.textures_delta,
            vec![],
        );
    }
}

impl Drop for EguiWindow {
    fn drop(&mut self) {
        // 显式解绑 surface:关窗口要真的把 GPU 资源还回去,而不是留着等下次开
        self.painter.on_window_event_destroyed(egui::ViewportId::ROOT);
    }
}
```

`crates/desktop/src/lib.rs` 追加 `pub mod window;`。

- [ ] **Step 3: 编译并按编译器修正**

Run: `cargo build -p octoterm-desktop`

预期会有若干签名不匹配。逐个按 `cargo doc -p egui-wgpu -p egui-winit --open` 修正。
**判定标准是对外接口不变**:`open` / `id` / `on_window_event` / `redraw` /
`request_redraw` 的签名必须与 Interfaces 一致,Task 8 依赖它们。

- [ ] **Step 4: 接进 app.rs,点「设置…」开一个空窗口**

`App` 加字段 `settings_window: Option<crate::window::EguiWindow>`,并:

```rust
            UserEvent::MenuClicked(MenuAction::Settings) => {
                if self.settings_window.is_none() {
                    match crate::window::EguiWindow::open(event_loop, "octoterm 设置", (460, 420)) {
                        Ok(w) => self.settings_window = Some(w),
                        Err(e) => tracing::error!(error = %e, "无法打开设置窗口"),
                    }
                }
                if let Some(w) = &self.settings_window {
                    w.request_redraw();
                }
            }
```

`window_event` 改成:

```rust
    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(w) = self.settings_window.as_mut().filter(|w| w.id() == id) else {
            return;
        };
        let _consumed = w.on_window_event(&event);
        match event {
            WindowEvent::CloseRequested => {
                // 关窗口只是关窗口,程序继续常驻。Drop 会把 GPU 资源还回去。
                self.settings_window = None;
            }
            WindowEvent::RedrawRequested => {
                w.redraw(|ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.label("设置界面将在下一步实现");
                    });
                });
            }
            _ => {
                w.request_redraw();
            }
        }
    }
```

- [ ] **Step 5: 手动验收**

Run: `cargo run -p octoterm-desktop`

1. 点「设置…」,弹出窗口,里面有那行占位文字
2. 关掉窗口,程序仍然常驻,托盘还在
3. 再点「设置…」,窗口能重新打开(证明 Drop 之后还能重建)
4. 窗口开着的时候拖动缩放,内容跟着重绘不花屏
5. 用活动监视器 / 任务管理器看:关掉窗口后内存回落

- [ ] **Step 6: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 按需创建/销毁的 egui + wgpu 窗口"
```

---

### Task 8: 设置界面与保存流程

**Files:**
- Create: `crates/desktop/src/settings/ui.rs`
- Modify: `crates/desktop/src/settings/mod.rs`、`crates/desktop/src/app.rs`

**Interfaces:**
- Consumes: `settings::state::{Form, FieldError, needs_rebind}`、`configfile::save`、`supervisor::Supervisor`、`autostart`
- Produces:
  - `settings::ui::View { pub form: Form, pub window_size: String, pub launchers: Vec<(String, String)>, pub message: Option<Message> }`
  - `settings::ui::Message { Ok(String), Err(String) }`
  - `settings::ui::Outcome { None, Save, Cancel, OpenConfigFile, RegenerateToken }`
  - `settings::ui::draw(ctx: &egui::Context, view: &mut View) -> Outcome`

`draw` 只画和收集意图,**不做任何 IO** —— 保存的副作用全在 `app.rs` 里。这样 UI 层
始终是纯的,不需要测。

- [ ] **Step 1: 实现 settings/ui.rs**

```rust
//! 设置窗口的绘制。只画界面、只返回用户意图,不碰文件、不碰 supervisor ——
//! 副作用全部由 app.rs 执行,UI 层因此保持无状态、无 IO。

use crate::settings::state::Form;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Ok(String),
    Err(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    None,
    Save,
    Cancel,
    OpenConfigFile,
    RegenerateToken,
}

pub struct View {
    pub form: Form,
    /// 只读展示,v1 不提供修改。
    pub window_size: String,
    /// (名称, 命令) —— 只读列表。
    pub launchers: Vec<(String, String)>,
    pub message: Option<Message>,
}

pub fn draw(ctx: &egui::Context, view: &mut View) -> Outcome {
    let mut outcome = Outcome::None;

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add_space(4.0);

        egui::Grid::new("settings").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label("监听地址");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut view.form.host).desired_width(140.0));
                ui.label(":");
                ui.add(egui::TextEdit::singleline(&mut view.form.port).desired_width(60.0));
            });
            ui.end_row();

            ui.label("访问 token");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut view.form.token).desired_width(180.0));
                if ui.button("重新生成").clicked() {
                    outcome = Outcome::RegenerateToken;
                }
            });
            ui.end_row();

            ui.label("");
            ui.small("留空表示不固定:每次启动随机生成。改动在下次启动生效。");
            ui.end_row();

            ui.label("会话尺寸策略");
            ui.horizontal(|ui| {
                ui.label(&view.window_size);
                ui.small("(在 config.toml 中修改)");
            });
            ui.end_row();

            ui.label("开机自启");
            ui.checkbox(&mut view.form.autostart, "");
            ui.end_row();
        });

        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("启动项");
            if ui.button("打开 config.toml").clicked() {
                outcome = Outcome::OpenConfigFile;
            }
        });
        egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
            if view.launchers.is_empty() {
                ui.small("(只有内置项)");
            }
            for (name, command) in &view.launchers {
                ui.horizontal(|ui| {
                    ui.label(name);
                    ui.small(command);
                });
            }
        });

        ui.add_space(8.0);
        if let Some(msg) = &view.message {
            match msg {
                Message::Ok(t) => ui.colored_label(egui::Color32::from_rgb(60, 140, 60), t),
                Message::Err(t) => ui.colored_label(egui::Color32::from_rgb(180, 60, 60), t),
            };
        }

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("取消").clicked() {
                outcome = Outcome::Cancel;
            }
            if ui.button("保存并应用").clicked() {
                outcome = Outcome::Save;
            }
        });
    });

    outcome
}
```

`crates/desktop/src/settings/mod.rs`:

```rust
pub mod state;
pub mod ui;
```

- [ ] **Step 2: 在 app.rs 里接上保存流程**

`App` 加字段 `view: Option<crate::settings::ui::View>`,并加这个方法:

```rust
    /// 打开设置窗口时,从当前生效值构造视图。
    fn build_view(&self) -> crate::settings::ui::View {
        use crate::settings::{state::Form, ui::View};
        let listen = self.sup.listen().unwrap_or(self.config.listen);
        let mut form = Form::from_current(listen, autostart_or_false());
        // token 回填当前生效值:用户多半是来看它、复制它的,展示为空反而费解
        form.token = self.sup.token().unwrap_or_default().to_string();
        View {
            form,
            window_size: format!("{:?}", self.config.window_size).to_lowercase(),
            launchers: self
                .config
                .launchers
                .iter()
                .map(|l| (l.name.clone(), l.command.join(" ")))
                .collect(),
            message: None,
        }
    }
```

`autostart_or_false()`:

```rust
fn autostart_or_false() -> bool {
    // 读不出来就当没开:这一项读失败不该阻止用户打开设置窗口
    crate::autostart::is_enabled().unwrap_or(false)
}
```

`App` 需要新增字段 `config: octoterm_server::config::Config`(由 `main.rs` 传入,
用于只读展示 `window_size` 与 `launchers`),`App::new` 的签名相应扩展为:

```rust
    pub fn new(
        rt: Runtime,
        sup: Supervisor,
        proxy: EventLoopProxy<UserEvent>,
        log_path: std::path::PathBuf,
        config: octoterm_server::config::Config,
    ) -> Self
```

保存的执行:

```rust
    /// 保存顺序见设计文档:先 bind(或先写文件)、任一步失败都不留中间状态。
    fn apply_settings(&mut self) {
        use crate::settings::{state::needs_rebind, ui::Message};

        let Some(view) = self.view.as_mut() else { return };

        let next = match view.form.validate() {
            Ok(v) => v,
            Err(e) => {
                view.message = Some(Message::Err(e.to_string()));
                return;
            }
        };

        // 开机自启与 server 无关,单独处理,失败只提示不阻断
        if let Err(e) = crate::autostart::set(view.form.autostart) {
            view.message = Some(Message::Err(format!("开机自启设置失败:{e}")));
            return;
        }

        let current_listen = self.sup.listen();
        let current_token = self.sup.token().unwrap_or_default().to_string();
        let rebind = match current_listen {
            Some(listen) => needs_rebind(listen, &current_token, &next),
            None => true, // 当前没在监听,无论如何都要试着起来
        };

        let path = match crate::configfile::default_path() {
            Ok(p) => p,
            Err(e) => {
                view.message = Some(Message::Err(format!("{e}")));
                return;
            }
        };

        if !rebind {
            // 只改了 token 的固定与否 / 只改了自启:写文件就完事
            match crate::configfile::save(&path, &next) {
                Ok(()) => view.message = Some(Message::Ok("已保存".into())),
                Err(e) => view.message = Some(Message::Err(format!("{e}"))),
            }
            return;
        }

        let token = next.token.clone().unwrap_or(current_token);
        let sessions = self.sup.manager().list().len();
        match self.rt.block_on(self.sup.restart(next.listen, token)) {
            Ok(actual) => {
                if let Err(e) = crate::configfile::save(&path, &next) {
                    // 已经在新地址上跑起来了,但配置没落盘 —— 必须说清楚
                    view.message = Some(Message::Err(format!(
                        "已在 {actual} 生效,但写入配置失败:{e}(重启后会回到旧配置)"
                    )));
                } else {
                    view.message = Some(Message::Ok(format!(
                        "已生效 · {actual} · {sessions} 个会话未受影响"
                    )));
                }
                self.refresh_status();
            }
            Err(e) => view.message = Some(Message::Err(format!("{e}"))),
        }
    }
```

> 注意这里的顺序与设计文档略有出入:设计文档写的是「先 bind、再写文件、再切换」,
> 但 `Supervisor::restart` 把 bind 与切换封装成了一步(它内部已经保证地址变化时
> 先 bind 后关)。因此 app 层的顺序是「restart → 写文件」,失败语义仍然满足:
> restart 失败时什么都没动,写文件失败时如实告诉用户"跑起来了但没存下"。

`window_event` 里 `RedrawRequested` 分支改成:

```rust
            WindowEvent::RedrawRequested => {
                use crate::settings::ui::{draw, Outcome};
                let mut outcome = Outcome::None;
                if let (Some(w), Some(view)) = (self.settings_window.as_mut(), self.view.as_mut()) {
                    w.redraw(|ctx| outcome = draw(ctx, view));
                }
                match outcome {
                    Outcome::None => {}
                    Outcome::Save => self.apply_settings(),
                    Outcome::Cancel => {
                        self.settings_window = None;
                        self.view = None;
                    }
                    Outcome::OpenConfigFile => {
                        if let Ok(p) = crate::configfile::default_path() {
                            let _ = open::that_detached(p);
                        }
                    }
                    Outcome::RegenerateToken => {
                        if let Some(view) = self.view.as_mut() {
                            view.form.token = uuid::Uuid::new_v4().simple().to_string();
                        }
                    }
                }
            }
```

`Outcome::Save` 那一步会 `block_on`,主线程会短暂卡住 —— 对一次点击是可接受的,
换来的是"保存流程完全串行、不需要任何中间态"。

`crates/desktop/Cargo.toml` 追加 `uuid = { version = "1", features = ["v4"] }`。

打开窗口时同时建视图:

```rust
            UserEvent::MenuClicked(MenuAction::Settings) => {
                if self.settings_window.is_none() {
                    self.view = Some(self.build_view());
                    match crate::window::EguiWindow::open(event_loop, "octoterm 设置", (460, 460)) {
                        Ok(w) => self.settings_window = Some(w),
                        Err(e) => {
                            tracing::error!(error = %e, "无法打开设置窗口");
                            self.view = None;
                        }
                    }
                }
                if let Some(w) = &self.settings_window {
                    w.request_redraw();
                }
            }
```

`main.rs` 里 `App::new` 的调用补上 `config`(注意 `config` 在 `Supervisor::new` 里
被借用过,传给 `App::new` 前先 `clone()`)。

- [ ] **Step 3: 手动验收**

Run: `cargo run -p octoterm-desktop`

1. 开一个会话,再打开设置
2. 把端口从 7683 改成 9000,点「保存并应用」→ 提示「已生效 · 127.0.0.1:9000 · 1 个会话未受影响」
3. 托盘状态行随之变成 9000
4. 浏览器里原来的页面重连后**会话内容还在**(这是整个设计的核心承诺)
5. 打开 `config.toml`,确认 `listen` 变了、手写的注释还在
6. 把 host 改成 `not-an-ip`,点保存 → 红字报「不是合法的 IP 地址」,且服务没有中断
7. 起一个别的程序占住 9001,把端口改成 9001 保存 → 红字报无法监听,**原来的 9000 仍然可用**
8. 勾上「开机自启」保存,检查 `~/Library/LaunchAgents/com.octoterm.desktop.plist`(或 HKCU Run 键)出现;取消勾选保存,它消失

- [ ] **Step 4: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 设置界面与保存流程"
```

---

### Task 9: 失败路径与退出确认

**Files:**
- Modify: `crates/desktop/src/main.rs`、`crates/desktop/src/app.rs`、`crates/desktop/src/settings/ui.rs`

**Interfaces:**
- Consumes: 前面全部
- Produces:
  - `app::Startup { pub config: Config, pub config_error: Option<String>, pub listen_error: Option<String> }`
  - `settings::ui::View` 新增字段 `pub banner: Option<String>`

- [ ] **Step 1: 让配置解析失败不再吞掉**

`main.rs` 里 `Config::load(None).unwrap_or_default()` 改成:

```rust
    // 配置坏了绝不能让 GUI 消失 —— 用户正是要靠这个 GUI 去修它
    let (config, config_error) = match Config::load(None) {
        Ok(c) => (c, None),
        Err(e) => {
            tracing::error!(error = %e, "配置文件解析失败,先用默认值起来");
            (Config::default(), Some(e.to_string()))
        }
    };
```

`sup.restart` 的失败同样记下来:

```rust
    let listen_error = match rt.block_on(sup.restart(config.listen, token)) {
        Ok(addr) => {
            tracing::info!(%addr, "已监听");
            None
        }
        Err(e) => {
            tracing::error!(error = %e, "启动时监听失败");
            Some(e.to_string())
        }
    };
```

两者与 `config` 一起打包成 `Startup`,**替换掉 Task 8 给 `App::new` 加的那个
`config` 参数**(`Startup` 已经含着它):

```rust
#[derive(Debug, Clone)]
pub struct Startup {
    pub config: octoterm_server::config::Config,
    pub config_error: Option<String>,
    pub listen_error: Option<String>,
}
```

```rust
    pub fn new(
        rt: Runtime,
        sup: Supervisor,
        proxy: EventLoopProxy<UserEvent>,
        log_path: std::path::PathBuf,
        startup: Startup,
    ) -> Self
```

`App` 的字段 `config: Config` 相应换成 `startup: Startup`,`build_view` 里三处
`self.config.…` 改成 `self.startup.config.…`(`listen` / `window_size` / `launchers`)。
`main.rs` 里 `App::new` 的调用同步改成传 `Startup { config: config.clone(), config_error, listen_error }`。

- [ ] **Step 2: 启动异常时自动弹设置窗口**

`App` 加字段 `startup: Startup`。`resumed` 末尾追加:

```rust
        // 起不来的时候必须主动把窗口推到用户面前:这时候 Web UI 是打不开的,
        // 设置窗口是唯一的出路。
        if self.startup.config_error.is_some() || self.startup.listen_error.is_some() {
            self.proxy
                .send_event(UserEvent::MenuClicked(MenuAction::Settings))
                .ok();
        }
```

`build_view` 里把两个错误合成横幅:

```rust
        let banner = match (&self.startup.config_error, &self.startup.listen_error) {
            (Some(c), Some(l)) => Some(format!("配置文件有错:{c}\n监听失败:{l}")),
            (Some(c), None) => Some(format!("配置文件有错(当前使用默认值):{c}")),
            (None, Some(l)) => Some(format!("当前未监听:{l}")),
            (None, None) => None,
        };
```

`settings/ui.rs` 的 `View` 加 `pub banner: Option<String>`,`draw` 在最顶部画:

```rust
        if let Some(banner) = &view.banner {
            ui.colored_label(egui::Color32::from_rgb(180, 60, 60), banner);
            ui.add_space(6.0);
        }
```

`status_text()` 里未监听的分支改成带原因:

```rust
            None => match &self.startup.listen_error {
                Some(e) => format!("octoterm · 未监听({e})"),
                None => "octoterm · 未监听".to_string(),
            },
```

- [ ] **Step 3: 退出前确认**

内嵌模型下退出会杀掉全部 pty 会话 —— 而"会话在断连后存活"正是这个项目的卖点,
所以这个动作必须显眼。`MenuAction::Quit` 分支改成:

```rust
            UserEvent::MenuClicked(MenuAction::Quit) => {
                let n = self.sup.manager().list().len();
                if n > 0 && !confirm_quit(n) {
                    return;
                }
                self.sup.stop();
                event_loop.exit();
            }
```

`confirm_quit` 用系统原生对话框,不占一个 egui 窗口。加依赖:

```toml
rfd = { version = "0.15", default-features = false }
```

```rust
/// 有活跃会话时确认;没有会话就别打扰用户。
fn confirm_quit(sessions: usize) -> bool {
    rfd::MessageDialog::new()
        .set_title("退出 octoterm")
        .set_description(format!(
            "还有 {sessions} 个会话正在运行。退出会终止它们,里面跑的程序都会被杀掉。"
        ))
        .set_buttons(rfd::MessageButtons::OkCancelCustom("退出".into(), "取消".into()))
        .show()
        == rfd::MessageDialogResult::Custom("退出".into())
}
```

> `rfd` 0.15 的 `MessageDialogResult` 变体请以 `cargo doc -p rfd --open` 为准;
> 语义固定为「点『退出』返回 true,其余一律返回 false」。default-features 关掉是
> 为了避开 GTK,macOS/Windows 上走原生对话框。

- [ ] **Step 4: 手动验收**

1. 把 `config.toml` 写成 `listen = = =`,启动 → 托盘出现,设置窗口自动弹出,红字显示 toml 错误
2. 修好保存,提示已生效
3. 用别的程序占住 7683,启动 → 托盘出现,状态行「未监听(…)」,设置窗口自动弹出;把端口改成 7684 保存 → 恢复正常
4. 开两个会话,点「退出」→ 弹确认框写明「还有 2 个会话」;点「取消」程序继续跑,会话还在
5. 杀掉全部会话再点「退出」→ 不弹框,直接退出

- [ ] **Step 5: 提交**

```bash
git add crates/desktop
git commit -m "feat(desktop): 启动失败不再让 GUI 消失,退出前确认会话"
```

---

### Task 10: 打包与发布

**Files:**
- Create: `scripts/bundle-macos.sh`
- Create: `scripts/bundle-windows.bat`
- Modify: `.github/workflows/release.yml`
- Modify: `README.md`、`README-cnzh.md`

- [ ] **Step 1: macOS 打包脚本**

`scripts/bundle-macos.sh`:

```bash
#!/usr/bin/env bash
# 把 octoterm-desktop 组装成一个 .app。
# LSUIElement=1 是关键:没有它,一个托盘常驻程序会在 Dock 里留个图标、
# 还会接管顶部菜单栏。
set -euo pipefail

TARGET="${1:-aarch64-apple-darwin}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="$ROOT/target/bundle/octoterm.app"
BIN="$ROOT/target/$TARGET/release/octoterm-desktop"

[ -f "$BIN" ] || { echo "找不到 $BIN,先跑 cargo build --release --target $TARGET -p octoterm-desktop" >&2; exit 1; }

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/octoterm-desktop"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>octoterm</string>
    <key>CFBundleDisplayName</key>
    <string>octoterm</string>
    <key>CFBundleIdentifier</key>
    <string>com.octoterm.desktop</string>
    <key>CFBundleExecutable</key>
    <string>octoterm-desktop</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

echo "$APP"
```

`chmod +x scripts/bundle-macos.sh`

- [ ] **Step 2: Windows 打包脚本**

`scripts/bundle-windows.bat`:

```bat
@echo off
REM Windows 侧不需要 bundle:单个 exe 就是全部产物(图标已嵌进二进制)。
REM 这个脚本只负责把它挪到统一的输出目录,和 macOS 那边对齐。
setlocal
set TARGET=%1
if "%TARGET%"=="" set TARGET=x86_64-pc-windows-msvc
set ROOT=%~dp0..
set BIN=%ROOT%\target\%TARGET%\release\octoterm-desktop.exe
if not exist "%BIN%" (
  echo 找不到 %BIN%,先跑 cargo build --release --target %TARGET% -p octoterm-desktop 1>&2
  exit /b 1
)
if not exist "%ROOT%\target\bundle" mkdir "%ROOT%\target\bundle"
copy /Y "%BIN%" "%ROOT%\target\bundle\octoterm-desktop.exe" >nul
echo %ROOT%\target\bundle\octoterm-desktop.exe
```

- [ ] **Step 3: release.yml 加 desktop 产物**

在 `release.yml` 的 build job 里,**只对 Windows 与 macOS 两个 target** 追加:

```yaml
      - name: Build desktop (windows/macos only)
        if: matrix.target == 'x86_64-pc-windows-msvc' || matrix.target == 'aarch64-apple-darwin'
        run: cargo build --release --target ${{ matrix.target }} -p octoterm-desktop

      - name: Bundle desktop (macOS)
        if: matrix.target == 'aarch64-apple-darwin'
        run: |
          ./scripts/bundle-macos.sh ${{ matrix.target }}
          cd target/bundle && zip -r octoterm-desktop-${{ matrix.target }}.zip octoterm.app

      - name: Bundle desktop (Windows)
        if: matrix.target == 'x86_64-pc-windows-msvc'
        shell: cmd
        run: scripts\bundle-windows.bat ${{ matrix.target }}
```

产物上传步骤按 `release.yml` 里既有的写法追加这两个文件。

- [ ] **Step 4: 更新 README**

`README.md` 与 `README-cnzh.md` 各加一节,放在 Quick start 之后:

英文版:

````markdown
## Desktop app (Windows / macOS)

`octoterm-desktop` runs the same server embedded in a tray-resident process,
with a small native settings window. It is not a terminal client — terminals
still live in the browser.

```sh
cargo run -p octoterm-desktop
```

Changing the listen address or token from the settings window rebuilds only the
HTTP layer: **your running sessions are not affected**. Quitting the app does
terminate them, so it asks first.

Linux is not supported.
````

中文版对应一节,措辞与英文版一致。

- [ ] **Step 5: 验证并提交**

Run:
```bash
cargo build --release -p octoterm-desktop
./scripts/bundle-macos.sh   # macOS 上
open target/bundle/octoterm.app
```
Expected: 双击 .app 后托盘出现,Dock 里**没有**图标

```bash
git add scripts .github/workflows/release.yml README.md README-cnzh.md
git commit -m "build(desktop): macOS .app 打包与发布产物"
```

---

## 完成标准

全部 task 做完后,以下每一条都要成立:

1. `cargo test --workspace` 在 macOS 与 Windows 上全绿;Linux 上 `--exclude octoterm-desktop` 全绿
2. `cargo clippy --workspace -- -D warnings` 干净
3. `crates/server` 除注释、以及一个独立的 `collapsible_if` lint 修复提交外零改动
   —— 用 `git diff master -- crates/server/src` 确认(那个 lint 提交是 rustc 1.95
   对既有代码的新告警,与 desktop 无关,只是为了让 clippy 门禁真的能用)
4. 改端口保存后,浏览器里的会话内容仍在
5. 配置文件里手写的注释在保存后一字不差
6. macOS 上 Dock 无图标、菜单栏图标随亮暗色反转
7. 配置写坏、端口被占,GUI 都照常可用
