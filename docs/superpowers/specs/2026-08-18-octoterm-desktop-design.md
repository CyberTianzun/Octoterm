# octoterm desktop(托盘常驻 GUI)设计

日期:2026-08-18
状态:已定稿(用户批准)

## 目标

给 octoterm 增加一个桌面常驻程序:一个托盘图标 + 一个设置窗口,只支持 Windows
与 macOS。它不是终端客户端 —— 终端仍然在浏览器里(或将来的原生客户端)。它解决
的是「daemon 怎么在桌面上被启动、被看见、被配置」。

非目标:Linux(winit/egui 能编,但不在支持范围,CI 也不为它花时间)、终端渲染、
配置热重载之外的任何服务端功能。

## 渲染方案的选型

候选三个,最终选 **winit + egui + tray-icon**。

### 为什么不是 gpui

`gpui` 已于 2025-10 发布到 crates.io(0.2.2,Apache-2.0),不再需要 git 依赖 zed
仓库,这一点上它没有问题。排除它的理由有三条:

1. **托盘是空白**。gpui 没有托盘 API,也没有暴露 macOS 的 activation policy
   (托盘常驻应用必须是 Accessory,否则占 Dock)。macOS 上可以自己用 objc2 塞
   NSStatusItem;Windows 上 gpui 跑自己的消息循环,能否和 `tray-icon` 需要的
   线程消息泵配合没有文档、需要试。而这两件事恰好就是托盘常驻应用的全部脏活,
   在 winit 生态里是铺好的路:`tray-icon`(Tauri 团队出品)官方文档就写了 winit
   集成,`ActivationPolicy::Accessory` 是 `EventLoopBuilderExtMacOS` 一行的事。
2. **足迹**。gpui 0.2.2 有 65 个直接的非可选依赖(egui 12 / eframe 22 / tray-icon
   12),其中包括 resvg+usvg(完整 SVG 渲染器)、lyon(路径细分)、taffy(布局
   引擎)、image,以及 smol —— 它自带一套 async runtime,会和本项目的 tokio 并存。
   这些是为 Zed 那个量级的编辑器准备的,对一个五六个字段的设置窗口是纯负担,与
   项目「small / memory-frugal」的定位相反。
3. **它不自带 settings 封装**。Zed 的 `settings` crate 没有单独发布,且与 Zed 的
   schema 体系深度耦合。crates.io 上相关的只有第三方的 `gpui-form`。

代价是 egui 自绘、非原生控件,macOS 上看得出来。对一个五六个字段的设置窗口接受。

### 为什么不是「只做托盘,设置页复用 Web UI」

最省的方案:托盘菜单打开浏览器,服务端配置做成 Web 客户端里的一个 tab,二进制和
内存几乎不变(Syncthing / qBittorrent 的形态)。

它有一个死结:**端口被占用、监听失败的时候,恰恰打不开那个用来改端口的页面**。
改 token 有同样的自锁风险。这是必须有原生窗口的决定性理由,也直接决定了下面
「GUI 的可用性不依赖 server 起没起来」这条规则。

### 为什么不是 Tauri

它的核心优势就是上面那条 Web 路线,却要额外背 wry/WebView2 和一套 tauri-cli 构建
流程;它的另一个卖点「托盘 + 窗口整合」已被 winit 方案覆盖。

### 为什么不用 eframe

eframe 假定自己拥有事件循环并在启动时就创建窗口,而托盘常驻应用要的是「启动时
0 窗口、关窗口时真正销毁 GPU 上下文」。直接驱动 winit 多一点管线代码,换来空闲时
进程里只剩 tokio 与一个状态栏图标。

## 进程模型:内嵌

desktop crate 直接依赖 `octoterm-server` 这个 lib,自己起 tokio runtime 调
`serve()`。一个二进制、一个进程,符合项目「single binary, one process」的定位。
CLI 二进制 `octoterm-server` 继续独立存在,不受影响。

主线程跑 winit 事件循环,tokio runtime 跑在后台线程。两者之间只有两条通道:winit
的 `EventLoopProxy`(server → UI 推状态)和一个 mpsc(UI → server 下命令)。

## 关键结论:「重启生效」基本不存在

`SessionManager` 与 HTTP 层完全解耦 —— `serve(listener, AppState)` 只是个 future,
pty 会话全在 `Arc<SessionManager>` 里。因此改配置的正确做法不是重启进程,而是:

```
停掉旧的 axum → 用新 listener + 新 AppState 再 spawn 一个
```

**不能用 axum 的 graceful shutdown**:它会等所有连接结束,而 WebSocket 是长连接,
永远不结束 —— graceful 在这里等于挂死。正确做法是 abort 掉那个 task、连同 listener
一起 drop。客户端本来就是为断线设计的(seamless resume),abort 就是这里的正确语义。

`SessionManager` 原地不动,**所有 pty 会话零损失**,客户端 WebSocket 断一下、按
既有的 seamless resume 自己接回来 —— 正是本项目本来就擅长的事。

按字段:

| 字段 | 生效方式 |
|---|---|
| `listen` | 重建 listener + 重启 HTTP 层。会话不丢 |
| `token` | 重建 AppState + 重启 HTTP 层。会话不丢 |
| `[[launcher]]` | 同上(`launchers` 就在 AppState 里)。会话不丢 |
| `window_size` | v1 不可配置(见下) |

`window_size` 是唯一的例外:它是 `SessionManager::new()` 的参数,并在
`Session::spawn` 时拷进每个 Session,运行时改需要动 SessionManager 的内部结构。
v1 决定**不让它可配置**,设置界面里只读展示当前值。

由此得到一个重要的边界收益:**`octoterm-server` 不需要任何行为改动**,
`serve` / `AppState` / `SessionManager` 本来就是 `pub` 的。只改两处措辞:
`config.rs` 里「只读加载,永不写文件」的注释,以及 README 中「配置文件从不自动
生成」的说法 —— 都改成「server 自己永不写,desktop 会写」。

## crate 结构

新 package `crates/desktop`,包名 `octoterm-desktop`,加入 workspace members。

```
crates/desktop/src/
  main.rs         winit 事件循环、ApplicationHandler、装配
  tray.rs         托盘图标与菜单(tray-icon / muda),菜单事件 → AppEvent
  supervisor.rs   内嵌 server 的生命周期:bind / serve / graceful shutdown / rebind
  settings/
    state.rs      设置状态、校验、脏标记、「这次改动要不要 rebind」—— 纯逻辑
    ui.rs         egui 绘制,只读 state
  configfile.rs   toml_edit 就地读写 config.toml
  logs.rs         tracing subscriber → 日志文件
crates/desktop/assets/icon.png   托盘图标,include_bytes! 嵌入
```

边界的含义:`settings/state.rs` 与 `configfile.rs` 不知道 egui 存在,
`supervisor.rs` 不知道 winit 存在。这三个是全部的可测逻辑,UI 层薄到不值得测。

依赖:`octoterm-server`、`winit` 0.30、`egui` 0.36 + `egui-winit` + `egui-wgpu`、
`tray-icon`、`muda`、`toml_edit`、`tokio`、`anyhow`、`tracing`、`tracing-subscriber`、
`directories`、`fs4`(单实例锁)、`png`(解托盘图标)、`winreg`(Windows 开机自启)。

渲染后端选 **egui-wgpu 而不是 egui_glow**:OpenGL 在 macOS 上自 10.14 起已废弃,
而 `glutin-winit`(egui_glow 在 winit 上建 GL 上下文所必需)最后一个版本停在
2024-06。wgpu 在 macOS 走 Metal、Windows 走 DX12,是受支持的原生路径 —— eframe
自己也正是为此把默认后端切到了 wgpu。

## 配置写入

用 `toml_edit` **就地修改**,只动用户在 UI 里改过的那几个 key,手写的注释、顺序、
空行全部原样保留;文件不存在时创建一个最小文件。不用 serde 序列化整个 `Config`
重写 —— 那会碾掉用户手写的注释和排版,对一个鼓励手写的配置文件不友好。

## 托盘

`muda` 构建菜单:

```
octoterm — 127.0.0.1:7683          (禁用项,状态行)
打开 Web 客户端
复制访问链接
──────────────
设置…
查看日志…
──────────────
退出
```

图标 tooltip 订阅 `SessionManager::events()`,显示
`octoterm · 127.0.0.1:7683 · 3 个会话`。用 tooltip 而不是菜单项,是因为菜单项要
在每次会话变化时重建整个菜单,tooltip 只是设一个字符串。

macOS 上托盘图标必须是 template image(纯黑 + alpha),用 `tray-icon` 的
`icon_is_template`,否则暗色菜单栏下是一坨黑块。

## 设置窗口

单页,不分 tab —— 可改的东西不足以撑起 tab。

| 项 | 形态 |
|---|---|
| 监听地址 | host + port 两个输入框,失焦时校验 |
| 访问 token | 输入框 +「重新生成」「复制」;留空的说明:每次启动随机生成 |
| 会话尺寸策略 | **只读**显示当前值,旁注「在 config.toml 中修改」 |
| 启动项 | **只读**列表(名称 + 命令)+「打开 config.toml」按钮 |
| 开机自启 | 勾选框 |
| | `[取消]` `[保存并应用]` |

真正可改的只有监听地址与 token。这看起来很少,但它恰好就是这个原生窗口的存在
理由:**这两项是「改错了就再也进不去 Web UI」的字段**,其余的东西编辑
`config.toml` 反而更顺手。启动项那种 name + argv 数组 + cwd 的结构化列表,做成
GUI 表格要花掉这个 crate 一半的代码量,换来的体验还不如直接开编辑器。

开机自启存在 desktop 自己的一份小配置里(与 `config.toml` 分开,因为它不是 server
的配置):macOS 写 LaunchAgent plist,Windows 写注册表 Run 键。

### 保存的顺序

**地址发生变化时**(最常见,也是最危险的一种):

```
1. 先 bind 新地址   ← 失败就到此为止,什么都没动,窗口里报错
2. 成功 → 写 config.toml(toml_edit 就地改)  ← 失败则 drop 新 listener,不变
3. abort 旧的 axum task
4. 用新 listener + 新 AppState 重新 spawn
5. 窗口提示「已生效 · N 个会话未受影响」
```

先 bind 后关保证了「端口被占用」这种最常见的失败不会把用户锁在外面 —— 旧的还在跑。

**地址没变、只改了 token 时**,先 bind 是做不到的:同一个地址上不能同时存在两个
listener(`SO_REUSEPORT` 在 Windows 上不可用,`SO_REUSEADDR` 在 Windows 上语义是
抢占,不能用)。这种情况改为「先写文件 → abort 旧 task → 带重试地 bind(端口刚
被自己释放,几次 20ms 的重试足够)」。万一仍然失败,进程进入「未监听」状态,托盘
状态行和设置窗口如实显示,用户可以改地址重试 —— 仍然不会自锁。

## 失败处理

**GUI 的可用性不依赖 server 起没起来。** 这是整个设计里最容易做错的地方:

- 端口被占用 → 托盘照常出现,状态行显示「未监听 · 7683 被占用」,并自动弹出设置窗口
- `config.toml` 解析失败 → 同样出托盘,设置窗口里显示原始的 toml 报错 +「打开 config.toml」

否则就退化成了上面排除掉的那个死结。

**退出会终止所有会话**。这是内嵌模型的直接代价:pty 子进程是这个进程的孩子,
进程没了会话就没了 —— 而「会话在断连后存活」正是本项目的卖点,所以退出这个动作
必须显眼。托盘「退出」在有活跃会话时弹确认框,写明「N 个会话将被终止」;没有会话
时直接退出,不打扰。关闭设置窗口只是关窗口,不退出程序。

**单实例**:两份 desktop 同时跑必然端口冲突。配置目录下一个 lock 文件,macOS
`flock`,Windows 命名 Mutex,第二个实例直接退出。v1 不做「唤起已有实例的窗口」——
那需要一条 IPC 通道,不值得。

**日志**:GUI 进程没有可见的 stderr(macOS 是双击 .app,Windows 是
`#![windows_subsystem = "windows"]`)。desktop 装自己的 tracing subscriber 写到配置
目录下的 `octoterm.log`,托盘「查看日志」用系统默认程序打开它。不做日志查看窗口
—— 一个只读文本框换一个 viewport 的复杂度,不划算。

## 打包

- **macOS**:脚本组装 `.app` bundle,`Info.plist` 里 `LSUIElement = 1`(不占 Dock、
  不接管菜单栏)。
- **Windows**:单个 `.exe`,`#![windows_subsystem = "windows"]` 去掉控制台窗口。
- 托盘图标 png 用 `include_bytes!` 嵌进二进制,与 `rust-embed` 嵌前端同一套思路。
- 打包脚本按现有 `build-frontend.sh` / `.bat` 的风格手写,不引入 `cargo-bundle`。

## CI

- `ci.yml` 的 ubuntu 那格改成 `cargo test --workspace --exclude octoterm-desktop`,
  clippy 同样处理;macOS / Windows 两格照旧全量。
- `release.yml` 只为 `x86_64-pc-windows-msvc` 与 `aarch64-apple-darwin` 产出 desktop
  产物。

## 测试

| 模块 | 测什么 |
|---|---|
| `configfile.rs` | 改 port 后手写注释与 `[[launcher]]` 段原样保留;文件不存在时创建最小文件;写失败时不留半个文件 |
| `settings/state.rs` | 校验(非法 IP、端口 0)、脏标记、一次改动要不要触发 rebind |
| `supervisor.rs` | `#[tokio::test]`:rebind 到新端口后,旧端口不再接受连接、新端口可以、**`SessionManager` 里的会话数不变** |
| `tray.rs` / `settings/ui.rs` | 不测 |

第三条是核心测试 —— 它直接把「改配置不丢会话」这个承诺钉死。可以借用既有的
`crates/server/tests/common/mod.rs` 里起测试服务器的脚手架。
