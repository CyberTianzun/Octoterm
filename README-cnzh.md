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
客户端中立的通讯协议暴露,今天可以在浏览器上操作,将来可以在手机和原生应用上
操作。

## 哲学

一把省内存的瑞士军刀,并且坚持只做一把瑞士军刀:

- **小巧**:单一静态二进制、单进程、常驻内存占用极小;
- **克制**:用最小功能集满足需求,把它做好;
- **不做 all-in-one**:窗口管理属于客户端;被托管的 shell 本身能干的事
  (编辑器、文件工具),不会重复造进 server。

## 快速开始

```sh
cd clients/web && npm install && npm run build && cd ../..
# 或者在仓库任意位置执行:./build-frontend.sh (Windows:build-frontend.bat)
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

对外监听请自行保证网络层安全(Tailscale / 反向代理 + TLS)。

## 桌面客户端(Windows / macOS)

`octoterm-desktop` 把同一套 server 内嵌进一个托盘常驻进程,配一个小巧的原生
设置窗口。它不是终端客户端——终端仍然长在浏览器里。

```sh
cargo run -p octoterm-desktop
```

在设置窗口里改监听地址或 token,只会重建 HTTP 层:**运行中的会话不受影响**。
但已经打开的页面不会跟着迁移——它的 WebSocket 根本没断,会继续在旧地址上工作,
直到你自己关掉那个页面;只有新连接才会走新地址。退出程序会终止所有会话,
所以有会话在跑时会先弹确认框。

不支持 Linux。

## 架构

通信协议规范:[`docs/protocol.md`](docs/protocol.md) —— 线上格式的规范性定义,
以及任何协议改动都要过的评审清单。设计背景与取舍:
`docs/superpowers/specs/2026-08-16-octoterm-design.md`。

- `crates/protocol` — 帧与消息定义(协议唯一事实来源)
- `crates/server` — daemon:pty、服务端 grid、WebSocket
- `crates/client-core` — Rust 客户端复用逻辑
- `crates/desktop` — 内嵌 server 的托盘常驻 GUI(Windows / macOS)
- `clients/web` — 参考客户端(TS + xterm.js)

## 路线图

octoterm 目前是**实验性 demo**,已完成的界面只有浏览器端。

1. **更多终端能力、更多客户端**:深化核心终端功能,然后把同一套协议带到移动
   端——iOS 与 Android 客户端;
2. **Agent 集成**:让被托管的会话可以在任何设备上接管 AI 的提示、回答它的选择、
   查看它的状态。**Claude Code 已可用**(装 hook、会话状态、远程授权);Codex、pi
   等其他 agent 在后续版本;
3. **不做文件管理**:文件管理基本不可能加——那是会话里 shell 自己的事。

## 已知限制

resync 恢复内容、光标与常用模式(应用光标键/括号粘贴/鼠标上报),不恢复
alt-screen 与滚动区域(DECSTBM);弱网重连后全屏应用建议 Ctrl-L 或触发重绘。

## 许可证

[MIT](LICENSE.md)
