# octoterm

English | [中文](README-cnzh.md)

A tiny, memory-frugal terminal server: a daemon hosts pty sessions that survive
disconnects, and clients attach over a WebSocket binary protocol with seamless
resume. It is not a tmux wrapper — it extracts the kernel of what tmux does
(process + IO + screen-state hosting) and gives window management back to the
client.

## Why

This project is inspired by tmux: a hosted, always-on console is something you
end up needing constantly. Jupyter's built-in terminal already nails the
experience — open a browser, get a live shell that survives the tab — but it is
deeply bound to Jupyter itself. What was missing is an **extremely lightweight,
memory-frugal standalone terminal server**.

That is octoterm's entire positioning. And unlike tmux, its GUI is not a pure
in-terminal UI: sessions are exposed through a client-neutral wire protocol, so
you can operate them from a browser today, and from phones and native apps
tomorrow.

## Philosophy

A memory-frugal Swiss Army knife — and it stays that way:

- **Small**: single static binary, one process, minimal resident footprint.
- **Restrained**: the minimum feature set that meets the need, done well.
- **Not all-in-one**: window management belongs to clients; anything a hosted
  shell can already do (editors, file tools) will not be rebuilt into the
  server.

## Quick start

```sh
cd clients/web && npm install && npm run build && cd ../..
cargo run -p octoterm-server
# The startup log prints a ready-to-open URL (Jupyter-style; a fresh random
# token is generated on every start):
#     http://127.0.0.1:7683/#token=<random token>
```

To pin a fixed token (so open pages survive server restarts), pass
`--token <value>`, or put `token = "<value>"` in a config file. The config file
is never auto-generated; create it yourself at
`~/Library/Application Support/octoterm/config.toml` (Linux:
`~/.config/octoterm/config.toml`), or point at one with `--config <path>`. All
fields are optional:

```toml
listen = "127.0.0.1:7683"
token = "my-fixed-token"
```

By default the server listens on 127.0.0.1 only. To reach it from other
devices, override on the command line (takes precedence over the config file):

```sh
cargo run -p octoterm-server -- --host 0.0.0.0 --port 9000
```

When exposing it beyond localhost, bring your own network-layer security
(Tailscale / reverse proxy + TLS).

## Architecture

Wire protocol: [`docs/protocol.md`](docs/protocol.md) — the normative spec, and
the checklist any protocol change has to pass. Background and rationale:
`docs/superpowers/specs/2026-08-16-octoterm-design.md`.

- `crates/protocol` — frame and message definitions (single source of truth)
- `crates/server` — the daemon: pty hosting, server-side grid, WebSocket
- `crates/client-core` — reusable logic for Rust clients
- `clients/web` — reference client (TypeScript + xterm.js)

## Roadmap

octoterm is currently an **experimental demo**, with the browser client as the
only finished surface.

1. **More terminal capability, more clients.** Deepen core terminal features,
   then bring the same protocol to mobile — iOS and Android clients.
2. **Agent integration.** Integrate coding agents such as Claude Code, Codex,
   pi and others, so a hosted session lets you take over an agent's prompts,
   answer its choices, and check its status from any device.
3. **No file manager.** File management is very unlikely to ever be added —
   that's what the shell in your session is for.

## Known limitations

Resync restores content, cursor and common modes (application cursor keys,
bracketed paste, mouse reporting), but not the alt screen or scroll regions
(DECSTBM). After a rough-network reconnect, full-screen apps may need a Ctrl-L
or an app-level redraw.

## License

[MIT](LICENSE)
