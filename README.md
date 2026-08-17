# octoterm

自研轻量终端会话内核:守护进程托管 pty 会话,浏览器(以及未来的原生客户端)
经 WebSocket 二进制协议 attach,断线无感续接。不是 tmux 的包装——是把
「进程 + IO + 屏幕状态托管」这一内核单独拿出来,把窗口管理还给客户端。

## 快速开始

```sh
cd clients/web && npm install && npm run build && cd ../..
cargo run -p octoterm-server
# 启动日志会打印完整访问 URL(Jupyter 式,token 每次启动随机生成),直接点开即可:
#     http://127.0.0.1:7683/#token=<本次随机 token>
```

要固定 token(重启后旧页面免重新登录),用 `--token <值>`,或在配置文件里写
`token = "<值>"`。配置文件不会自动生成;需要时自行创建
`~/Library/Application Support/octoterm/config.toml`(Linux 为
`~/.config/octoterm/config.toml`),或用 `--config <路径>` 指定,字段均可省略:

```toml
listen = "127.0.0.1:7683"
token = "my-fixed-token"
```

默认只监听 127.0.0.1。要在其他设备访问,用命令行参数覆盖(优先于配置文件):

```sh
cargo run -p octoterm-server -- --host 0.0.0.0 --port 9000
```

对外监听请自行保证网络层安全(Tailscale / 反向代理 + TLS)。

## 架构

见 `docs/superpowers/specs/2026-08-16-octoterm-design.md`。

- `crates/protocol` — 帧与消息定义(协议唯一事实来源)
- `crates/server` — daemon:pty、服务端 grid、WebSocket
- `crates/client-core` — Rust 客户端复用逻辑
- `clients/web` — 参考客户端(TS + xterm.js)

## 已知限制

resync 恢复内容、光标与常用模式(应用光标键/括号粘贴/鼠标上报),不恢复
alt-screen 与滚动区域(DECSTBM);弱网重连后全屏应用建议 Ctrl-L 或触发重绘。
