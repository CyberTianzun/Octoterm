# octoterm

自研轻量终端会话内核:守护进程托管 pty 会话,浏览器(以及未来的原生客户端)
经 WebSocket 二进制协议 attach,断线无感续接。不是 tmux 的包装——是把
「进程 + IO + 屏幕状态托管」这一内核单独拿出来,把窗口管理还给客户端。

## 快速开始

```sh
cd clients/web && npm install && npm run build && cd ../..
cargo run -p octoterm-server
# 记下打印的 token,浏览器打开:
# http://127.0.0.1:7683/#token=<token>
```

默认只监听 127.0.0.1。要在其他设备访问,编辑配置文件(路径见启动日志)里的
`listen`,并自行保证网络层安全(Tailscale / 反向代理 + TLS)。

## 架构

见 `docs/superpowers/specs/2026-08-16-octoterm-design.md`。

- `crates/protocol` — 帧与消息定义(协议唯一事实来源)
- `crates/server` — daemon:pty、服务端 grid、WebSocket
- `crates/client-core` — Rust 客户端复用逻辑
- `clients/web` — 参考客户端(TS + xterm.js)

## 已知限制

resync 恢复内容、光标与常用模式(应用光标键/括号粘贴/鼠标上报),不恢复
alt-screen 与滚动区域(DECSTBM);弱网重连后全屏应用建议 Ctrl-L 或触发重绘。
