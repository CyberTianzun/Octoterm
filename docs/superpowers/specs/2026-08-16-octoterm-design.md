# octoterm v1 设计

日期:2026-08-16
状态:已定稿(用户批准)

## 背景与定位

出发点是「tmux 作为终端托管服务,能否通过网络远程 attach」。调研结论:tmux 的 client-server 通信依赖 Unix domain socket 上的 `SCM_RIGHTS` fd 传递(client 把自己的 tty fd 直接交给 server),该机制无法跨网络,且其 wire protocol 是不稳定的内部实现,因此「转发 tmux socket」不可行。

经过路线对比(基于 tmux 包装 vs 自研),octoterm 确定为**自研的轻量终端会话内核**:

- 守护进程托管 pty 与子进程,会话在客户端断开后继续存活;
- 客户端(网页、原生 GUI)通过网络协议 attach;
- 窗口/标签/分栏管理完全属于客户端,服务端没有任何 UI 概念(这是与 tmux 的本质区别,tmux 的窗口管理对本项目是冗余);
- 不依赖 tmux,但不排斥——tmux 可以作为普通程序跑在被托管的 shell 里。

### 放弃 tmux 方案的理由

1. 宿主永远 Unix-only,而 Windows 原生宿主(ConPTY)是本项目一等目标;
2. tmux 的字符界面窗口管理与客户端自绘 UI 冲突,绕开它(control mode)实现量大增;
3. tmux 唯一不可替代的资产——服务端屏幕状态——如今可用 `alacritty_terminal` 以集成成熟 crate 的成本获得(shpool、zellij 已验证此路线)。

## 核心场景

单机个人工具:「个人终端云」。用户在任何设备(桌面浏览器、手机/平板浏览器、将来的原生 GUI)上重连到自己主机上的常驻终端会话。要解决的痛点,按优先级:

1. 浏览器/移动端可用(无 SSH 客户端的设备打开网页即可接上会话);
2. 会话管理体验(列表、预览、一键切换,而非盲敲命令);
3. 弱网/漫游体验(断线自动无感续接)。

明确排除:NAT 穿透/中继(假设局域网、Tailscale 或自有公网可直连)、多主机聚合、多用户。

## 架构

```
┌──────────────┐            ┌───────────────────────────────┐
│ web client   │─WebSocket─►│ octoterm-server (Rust daemon) │
│ (xterm.js)   │            │  ┌─ session ──────────────┐   │
├──────────────┤            │  │ pty (portable-pty)     │   │
│ rust gui     │─WebSocket─►│  │ child: shell/pwsh      │   │
│ (alacritty_  │            │  │ grid (alacritty_term.) │   │
│  terminal)   │            │  │ 环形缓冲 + seq         │   │
└──────────────┘            │  └────────────────────────┘×N │
                            └───────────────────────────────┘
                              Unix (openpty) / Windows (ConPTY)
```

单一 Rust 二进制,跑在被托管主机上,监听一个端口,同时承担:静态托管 web 客户端、控制协议、终端流代理。

## 仓库结构(cargo workspace)

- `octoterm-protocol` — 帧格式与消息类型定义,协议的唯一事实来源,server 与所有 Rust 客户端共享;
- `octoterm-server` — daemon:会话管理、pty 泵、grid 维护、WebSocket 服务、token 鉴权、静态资源内嵌(`rust-embed`);
- `octoterm-client-core` — Rust 客户端公共逻辑(连接、重连退避、seq/ack、resync 处理),为后续 GUI 客户端准备;
- `clients/web` — v1 参考客户端,vanilla TypeScript + xterm.js,无前端框架,构建产物内嵌进 server。

Rust GUI 客户端本体是后续独立项目,不在 v1,但协议与 client-core 的设计以它为消费者之一。

## 协议

设计原则:**客户端中立**。客户端只需要是一个哑 VT 渲染器(xterm.js、alacritty_terminal 或任何终端模拟器)加一个 JSON 控制通道,不需要理解服务端任何内部数据结构。

### 传输与帧

一条 WebSocket 连接承载全部流量,内部按 channel 多路复用。WS binary frame 内的帧格式:

```
[channel_id: u32 LE][flags: u8][payload...]
```

- `flags` 保留,v1 恒为 0;
- channel 0:控制通道,payload 为 JSON;
- 其他 channel:每个对应一个已 attach 的会话,payload 为原始 VT 字节流(双向:服务端→客户端为 pty 输出,客户端→服务端为键盘输入),不做任何再编码,服务端近乎零拷贝转发。

帧格式与传输层解耦:v1 只实现 WebSocket 承载,原生客户端将来可用同一帧格式跑裸 TCP/QUIC。

### 控制消息(channel 0,JSON)

- 会话 CRUD:`list-sessions`、`new-session`(可选 `command` argv,缺省为默认 shell)、`kill-session`、`rename-session`;
- `preview`:不 attach 的前提下获取某会话当前屏幕的重绘序列(会话列表页的卡片预览用);
- 生命周期:`attach`(分配 channel id,携带 `last_seq` 供续接)、`detach`、`resize {cols, rows}`;
- 服务端推送:会话增删改事件(会话列表保持活的)、`resync-begin`/`resync-end` 边界标记、会话退出通知;
- 握手:连接后第一条消息必须是 `hello`(携带 token 与协议版本),校验失败即断开。浏览器 WebSocket 无法自定义 header,故 token 走带内握手;静态资源不含秘密,不设防。

控制流量极小,JSON 换取 TS 端零成本解析;性能敏感路径(终端 IO)上没有 JSON。

### resync 也说 VT 语言

服务端需要向落后或重连的客户端恢复画面时,直接从权威 grid 生成一段 ANSI 重绘序列(清屏 + 光标定位 + 属性与内容重写 + 模式恢复),经会话 channel 推送,前后用控制消息标记边界。客户端收到 `resync-begin` 时重置本地终端状态即可。

## 会话状态与续接、流控

每会话服务端维护:

- `alacritty_terminal` 权威 grid(含滚动缓冲),持续消费 pty 输出;
- 带全局序号的原始输出字节环形缓冲,默认 1 MiB/会话。

行为:

- **正常路径**:原始字节流直达客户端;输出按 ~16ms 定时或缓冲阈值合帧,突发输出(如 `cat` 大文件)不会产生帧洪流;
- **断线重连**:客户端指数退避自动重连,`attach` 携带 `last_seq`;缺失区间在环形缓冲内 → 补发缺失字节(无缝);超出 → grid 重绘 resync(瞬间恢复现场,mosh 式有损恢复);
- **背压**:不引入显式 credit 消息。每会话输出走有界广播队列,每连接的发送受 WebSocket/TCP 背压;慢客户端导致队列 lag 时,服务端丢弃其错过的字节(grid 照常消费 pty),随后直接 resync 到最新画面。弱网客户端永远只落后「一屏」,不会被字节洪流压垮;
- **滚动历史**:正常流下由客户端本地积累;resync 是有损恢复,历史出现断点为接受的取舍(服务端回滚补页留给 v2)。

## 跨平台

- `portable-pty`(WezTerm 生态)统一 Unix openpty 与 Windows ConPTY,Windows 原生宿主为一等目标;
- 默认 shell:Unix 取 `$SHELL`(回退 `/bin/sh`),Windows 取 PowerShell;
- CI:macOS / Linux / Windows 三平台构建 + 会话生命周期集成测试;
- v1 以前台进程方式运行,服务化(systemd/launchd/Windows 服务)由用户自理,安装器不在 v1。

## Web 客户端(v1 参考实现)

两个界面:

1. **会话列表页**:卡片式,含屏幕预览(预览内容即 grid 渲染的重绘序列,前端用一个离屏/迷你 xterm.js 呈现)、新建/改名/杀死操作;
2. **终端页**:全屏 xterm.js + fit addon,顶部极简返回栏。

验收标准包含移动浏览器可用:触摸滚动、软键盘弹出时 resize 正确。

## 安全

- bearer token:配置文件指定或首次启动生成打印;WebSocket 连接经 `hello` 带内握手校验,失败即断开(静态资源不含秘密,公开);
- 默认只监听 `127.0.0.1`,对外监听需显式配置;文档引导 Tailscale / 反向代理加 TLS;
- TLS 终结不进 daemon。

## 测试

- `octoterm-protocol`:帧编解码与控制消息序列化单测;生成一致性夹具(JSON + 二进制样例)供 TS 端测试比对,保证两种语言实现的协议一致;
- `octoterm-server` 集成测试:会话 CRUD、attach/detach、resize、断线后环形缓冲补发、超窗 resync、credit 背压;
- grid 正确性往返测试:喂入 VT 序列 → 从 grid 生成重绘序列 → 喂入全新 emulator → 比对两个 grid 一致;
- web 客户端:桌面 + 移动浏览器手动验收清单。

## 明确不做(v1 YAGNI)

窗口/分栏语义、多主机聚合、多用户/权限、会话协作共享、NAT 穿透/中继、预测回显、QUIC/裸 TCP 传输、服务端滚动补页、服务安装器、Rust GUI 客户端本体。
