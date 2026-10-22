# Octoterm

[English](README.md) | 中文

极致轻量、省内存的 terminal server:守护进程托管 pty 会话,断线后会话继续存活,
客户端经 WebSocket 二进制协议 attach、无感续接。它不是 tmux 的包装——而是把
tmux 的内核(进程 + IO + 屏幕状态托管)单独拿出来,把窗口管理还给客户端。

## 为什么做这个

本项目受 tmux 启发:被托管的、常驻的控制台是日常刚需。Jupyter 内置的 terminal
其实已经把体验做得很好了——打开浏览器就是一个活着的 shell,关掉标签页也不丢——
但它和 Jupyter 深度绑定。缺的是一个**极其轻量、极其省内存的独立 terminal
server**。

这就是 octoterm 的全部定位。与 tmux 不同,它的 GUI 不是纯终端界面:会话通过
客户端中立的通讯协议暴露,可以在浏览器或 Android 原生客户端中操作。

## 哲学

一把省内存的瑞士军刀,并且坚持只做一把瑞士军刀:

- **小巧**:单一静态二进制、单进程、常驻内存占用极小;
- **克制**:用最小功能集满足需求,把它做好;
- **不做 all-in-one**:窗口管理属于客户端;被托管的 shell 本身能干的事
  (编辑器、文件工具),不会重复造进 server。

## 快速开始

```sh
# 需要 Node/npm:server 的 build.rs 会自动构建 clients/web
# (设置 OCTOTERM_SKIP_WEB_BUILD=1 可跳过)
cargo run -p octoterm-server
# 启动日志会打印完整访问 URL(Jupyter 式,token 每次启动随机生成),直接点开即可:
#     http://127.0.0.1:7683/#token=<本次随机 token>
```

要固定 token(重启后旧页面免重新登录),用 `--token <值>`,或在配置文件里写
`token = "<值>"`。配置文件不会由 server 自动生成;需要时自行创建
`~/Library/Application Support/octoterm/config.toml`(Linux 为
`~/.config/octoterm/config.toml`),或用 `--config <路径>` 指定,字段均可省略:

```toml
listen = "127.0.0.1:7683"
token = "my-fixed-token"
# 多端同时 attach 同一个会话时,pty 只有一个尺寸,各端的诉求需要归并:
# "smallest"(默认,谁都不会看到被截断的画面)、"largest"、"latest"(跟随最近一端)
window_size = "smallest"

# 给「新建会话」菜单加几条自定义项。可选 —— 菜单本来就有默认 shell,
# 以及从 iTerm2 / Windows Terminal 里扫出来的 profile。
[[launcher]]
name = "prod ssh"
command = ["ssh", "prod01"]   # 直接是 argv,不是命令行字符串:不用猜切分规则
cwd = "~/work"                # 可选
```

### 新建会话菜单

点 **+** 弹出的是一个菜单,不是输入框。菜单内容由服务端的 provider 提供,
常见情况下什么都不用配:

| provider | 读哪里 |
| --- | --- |
| 内置 | `$SHELL`(unix)/ `powershell.exe`、`%ComSpec%`(Windows) |
| 自定义 | 上面配置文件里的 `[[launcher]]` |
| iTerm2 | `com.googlecode.iterm2.plist` + `DynamicProfiles/`(macOS) |
| Windows Terminal | `settings.json`(商店版 / Preview / 便携版都找) |

第三方配置**只读不写**。只收「跑什么、在哪跑」跟默认不同的 profile ——
只改了配色的那些在这个菜单里是噪音。某个 provider 失败(没装、配置损坏、
读不了)只是少几条并留一行日志,菜单照常能用。

想接一个新来源,在 `crates/server/src/launcher/` 里实现一个 `LauncherProvider` 即可。

默认只监听 127.0.0.1。要在其他设备访问,用命令行参数覆盖(优先于配置文件):

```sh
cargo run -p octoterm-server -- --host 0.0.0.0 --port 9000
```

使用 `--host 0.0.0.0` 时,启动日志会列出回环地址及所有已连接网卡的 IPv4 访问链接
(含 token),包括局域网和 VPN 地址。

对外监听请自行保证网络层安全(Tailscale / 反向代理 + TLS)。

server 每次启动时会在后台检查 GitHub 最新正式 Release。发现更高版本时,在控制台
打印新版本号和 Release 页面链接;断网或检查失败不影响服务启动。

## 桌面客户端(Windows / macOS)

`octoterm-desktop` 把同一套 server 内嵌进一个托盘常驻进程,配一个小巧的原生
设置窗口。它不是终端客户端——终端仍然长在浏览器里。

```sh
cargo run -p octoterm-desktop
```

desktop 启动后也会在后台检查最新正式 Release。有更新时弹出系统对话框,
可选择「立刻更新」用默认浏览器打开 Release 页面,或「取消」继续使用。
程序不会自动下载或安装更新。

在设置窗口里改监听地址或 token,只会重建 HTTP 层:**运行中的会话不受影响**。
但已经打开的页面不会跟着迁移——它的 WebSocket 根本没断,会继续在旧地址上工作,
直到你自己关掉那个页面;只有新连接才会走新地址。退出程序会终止所有会话,
所以有会话在跑时会先弹确认框。

**聊天视图与它的暴露面**:托管会话里跑着 Claude Code 时,可以用聊天视图打开它 ——
结构化的消息流,而不是一块 VT 画面。它读的是 agent 自己写的对话记录,**只读、不缓存、
不落盘**,并且由 `agents.transcript_enabled` 单独门控(默认开,可在 `[agents]` 里设为 `false` 关掉)。

之所以和装 hook 分成两个开关:装 hook 让 octoterm 看到「它在干什么」,这个开关让它看到
「它说了什么」—— 后者是代码、路径、命令、模型的思考过程,暴露面高一个数量级,不该靠
前者顺带同意。**尤其要和下面这条一起看**:desktop 默认监听全网卡,而这两个开关默认都开着,
等于局域网里拿到 token 的人能读到你的整段对话 —— 在不信任的网络上请关掉它,或把监听改回 127.0.0.1。

**hook 自检与清理**:每次启动(desktop 每次改端口重启服务时也一样)都会检查已装的 hook 是否和
当前服务一致(端口、事件、程序路径),对不上就在控制台 / 日志里打警告 —— 只报不改,修复仍在
「设置 → AI 集成」里点「重新安装」。异常残留可以在同一页点「清空」,或运行
`octoterm-server purge-hooks`:删掉 agent 配置里**所有** octoterm 写过的 hook(不论端口、不论事件),
你自己的 hook 不受影响,改前备份。

**默认监听 `0.0.0.0:7683`(全网卡),与 `octoterm-server` 不同。** desktop 的存在理由
就是「从手机/平板连回自己这台机器」,只回环等于开箱即不可用。敢这么定的前提是两条硬
边界都在:空 token 进不到监听层(release 构建也拦),`/hook/*` 无条件只认回环。剩下的
残余风险是实打实的 —— 同一局域网里拿到 token 的人就能用你的终端,所以在不可信网络里
请到设置窗口改回 `127.0.0.1`,或自己套一层网络层安全(Tailscale / 反代 + TLS)。

不支持 Linux。

## Android 客户端

服务器连接由应用统一管理：从终端返回会话列表、再返回服务器列表，都保留服务器的
WebSocket。离开终端、切换到聊天或进入后台时会 detach 终端视口，让其他客户端恢复
自己的尺寸；重新打开终端时自动 attach。服务器列表显示各连接的状态，并提供重连和
手动断开入口。默认开启后台连接，通过前台服务及系统常驻通知维持；通知中可全部断开。
设置中可改为仅前台连接。Cloud 推送独立保留，用于服务器不可达或 Android 终止应用进程时的补充。

`clients/android` 是 Kotlin + Jetpack Compose 原生客户端,支持连接管理、服务端
启动项、多个会话、回滚与文字选择。界面和终端默认随系统切换 `2026 Light` /
`2026 Dark`;软键盘弹出时缩小终端视口,快捷键栏默认双行、可左右滑动。
首页右上角的设置统一管理语言、外观和快捷键行数,可切换单行 / 双行。按钮为 35dp 方形。
服务端识别出当前终端中的 Agent 后,快捷键自动切换为 Agent 布局,将 `P↑`、`P↓`
和多行输入键放在最前面。通过「设置 → Agent 快捷键布局 → 配置」可按 Agent 分别选择
`⌃J`、`⇧↵` 或两者,调整其他优先键的顺序,也可为某个 Agent 使用普通终端布局。

连接 Windows 服务端时,键盘适配层保留 Shift+Enter、Ctrl+J、Alt+方向键等组合键
的身份与修饰状态。快捷栏包含 Esc、Ctrl+C、方向键、Home、End、PgUp/PgDn 等。

使用 JDK 17 和 Android SDK 36 构建,或用 Android Studio 打开 `clients/android`:

```sh
cd clients/android
./gradlew :app:assembleDebug :app:testDebugUnitTest
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Windows 使用 `gradlew.bat`。安装后添加服务端地址和 token,也可以从剪贴板导入
服务端访问 URL。

## Agent 会话历史

Web 侧栏的历史按钮、Android 服务端页面的「Agent 会话历史」可浏览宿主机已有的
Claude Code、Codex、pi、Grok 和 Antigravity CLI 记录,包括在 Octoterm 之外开始的对话。
支持按 Agent、标题、原工作目录和会话 ID 搜索,并预览最近的对话。

已关联到运行中 Octoterm 终端的记录可直接进入终端;其他记录在原工作目录中启动一个
新的 Octoterm 终端并恢复。需要服务端 PATH 中有对应 CLI、原目录仍存在且记录包含恢复
所需信息。pi 使用原会话文件恢复。恢复请求在服务端合并,多端同时点击也不会重复启动。

历史读取沿用 `[agents].transcript_enabled` 和服务端 bearer token。服务端启动时异步
构建最多 1000 个去重会话的内存索引,客户端默认每页加载 50 条元数据,列表末尾提示上限。
对话正文只在预览时读取。刷新在后台重建索引,不阻塞分页请求;达到扫描上限或遇到坏记录
时界面会提示。当前读取
各 Agent 的默认用户目录,不扫描 WSL、其他用户目录或自定义存储位置。

## Octoterm Cloud(可选)

登录 Cloud 之后，agent 进入「等你」状态时会通过 Cloud 推送到手机（Android 构建需要配置推送）。不登录不影响任何功能。

- Cloud 不转发终端数据,拿不到本机的访问 token;推送只包含会话编号、agent 名称与工具名。
- 登录态保存在配置目录的 `cloud.json`,在 Unix 上权限仅属主可读写(0600)。
- 同一台机器上的 desktop 应用和独立运行的 octoterm-server 共用这份 cloud.json,其中一边登录或退出,另一边也会同步生效。

```bash
octoterm-server cloud register        # 注册并登录(邮件验证码)
octoterm-server cloud login
octoterm-server cloud status
octoterm-server cloud logout
octoterm-server cloud reset-password
octoterm-server cloud delete-account  # 永久删除账号及全部数据
```

Cloud 地址可写进 `config.toml`:

```toml
[cloud]
url = "https://cloud.example.com"
```

Desktop 用户在「设置…」窗口底部的「Cloud 账号」区选择中继地区，点击「通过浏览器登录」。
注册、登录和找回密码在浏览器中完成，成功后自动回调本机并保存登录状态，无需填写 Cloud 地址。

| 中继地区 | 地址 |
|---|---|
| 中国大陆地区（默认） | `https://octoterm.signal.top/` |
| 北美地区（暂未部署） | `https://octoterm.eclosion.ai/` |
| 其它全球（暂未部署） | `https://global.octoterm.eclosion.ai/` |

网页登录需要 Cloud 部署包含 `/auth/desktop` 与 `/v1/auth/desktop/*` 的版本。
Android 同样使用三个中继地区预设，并在浏览器中完成登录、注册和找回密码。
完成后自动返回 App，保存设备凭据并登记推送；需要 Cloud 部署 `/auth/mobile`
和 `/v1/auth/mobile/*` 接口。登录后账号图标着色并在右下角显示小勾，未登录时显示小叉。
账号页显示推送登记结果并提供重试；Cloud 通过部署参数选择阿里云或 FCM，Android 从
`/v1/push-config` 自动读取路线并登记对应设备。阿里云客户端配置放在
`clients/android/app/aliyun-emas-services.json`（不入库），其中的 App 配置与仅供服务端使用的
阿里云 AccessKey 分开。阿里云路线不依赖 Google Play 服务，FCM 需要可用的 Google Play
服务和能访问 Google 的网络；两者都需要通知权限。阿里云自有通道要求应用进程可运行，
进程被系统杀掉后及时展示还需要额外接入并配置厂商推送通道。
更新后的 Desktop/server 还会在 Stop hook 完成本轮任务时发送完成提醒。
登录等待最多 10 分钟，可在客户端取消；本机回调只监听 `127.0.0.1` 的临时端口。

## 架构

通信协议规范:[`docs/protocol.md`](docs/protocol.md) —— 线上格式的规范性定义,
以及任何协议改动都要过的评审清单。设计背景与取舍:
`docs/superpowers/specs/2026-08-16-octoterm-design.md`。

- `crates/protocol` — 帧与消息定义(协议唯一事实来源)
- `crates/server` — daemon:pty、服务端 grid、WebSocket
- `crates/client-core` — Rust 客户端复用逻辑
- `crates/desktop` — 内嵌 server 的托盘常驻 GUI(Windows / macOS)
- `clients/web` — 参考客户端(TS + xterm.js)
- `clients/android` — Android 原生终端客户端(Kotlin + Jetpack Compose)

## 路线图

octoterm 目前是**实验性 demo**,已提供浏览器端和 Android 原生客户端。

1. **更多终端能力、更多客户端**:深化核心终端功能,继续完善 Android 客户端,
   并把同一套协议带到 iOS 等平台;
2. **Agent 集成**:让被托管的会话可以在任何设备上接管 AI 的提示、回答它的选择、
   查看它的状态。**Claude Code 已可用**(装 hook、会话状态、远程授权、聊天视图);
   Codex 也已支持(状态、授权、聊天视图;hook 要在 Codex 里用 `/hooks` 审核一次);
   **pi** 通过生成的扩展 `~/.pi/agent/extensions/octoterm.ts` 支持(状态与聊天视图;
   pi 本身没有授权审批,所以没有远程授权可做);**Grok**(xAI 的 `grok`)通过
   `~/.grok/hooks/octoterm.json` 支持(状态与聊天视图;它同样没有授权审批事件);
   **Antigravity CLI**(Google 的 `agy`)通过共用的 `~/.gemini/config/hooks.json` 里的
   `octoterm` 条目支持(状态与聊天视图;不接管它的授权确认);
   该适配针对独立的 Antigravity CLI，与 Gemini CLI 和 Antigravity IDE 分开处理。
   从旧 Windows 版本升级后，需要在设置中重新安装 Antigravity 集成，再重启正在运行的
   `agy`，替换旧 hook 命令。聊天视图读取 Antigravity 的步骤日志，包含新版 CLI 的思考内容。
3. **不做文件管理**:文件管理基本不可能加——那是会话里 shell 自己的事。

## 已知限制

resync 恢复内容、光标与常用模式(应用光标键/括号粘贴/鼠标上报),不恢复
alt-screen 与滚动区域(DECSTBM);弱网重连后全屏应用建议 Ctrl-L 或触发重绘。

## 许可证

[MIT](LICENSE.md)
