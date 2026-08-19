# octoterm wire protocol

```
proto_version: 1        # octoterm_protocol::PROTO_VERSION
status:        normative
audience:      implementers; AI reviewers of protocol changes
scope:         everything on the wire between a client and octoterm-server
```

## 0. How to use this document

- Keywords MUST / MUST NOT / SHOULD / MAY are RFC 2119.
- Every rule has a stable ID (`T1`, `S7`, `R4`, …). Cite IDs in reviews, PRs and
  commit messages. IDs are append-only: never renumber, never reuse. A dropped
  rule keeps its ID and is marked `RETIRED`.
- This document is the contract; the code is one implementation of it. A
  divergence is a bug in exactly one of the two — fix it, do not fork the truth.
- Changing or adding a message: start at §12, not at the code.

## 1. Source of truth map

| concern | path |
| --- | --- |
| frame codec (Rust) | `crates/protocol/src/frame.rs` |
| frame codec (TS) | `clients/web/src/protocol.ts` |
| message types | `crates/protocol/src/messages.rs` |
| cross-language fixtures | `crates/protocol/fixtures/{client,server}-msgs.json` |
| handshake, JSON side-channel | `crates/server/src/app.rs` |
| control dispatch, output pumps | `crates/server/src/conn.rs` |
| launcher providers (discovery) | `crates/server/src/launcher/` |
| launcher client | `clients/web/src/launchers.ts`, `clients/web/src/new-session.ts` |
| pty, ring buffer, grid | `crates/server/src/session/{pty,buffer,grid}.rs` |
| geometry merge and policy | `crates/server/src/session/pty.rs`, `crates/server/src/config.rs` |
| client resume logic | `crates/client-core/src/lib.rs`, `clients/web/src/client.ts` |
| protocol integration tests | `crates/server/tests/ws_{auth,control,attach,geometry}.rs` |
| side-channel integration tests | `crates/server/tests/http_launchers.rs` |
| agent integration (server side) | `crates/server/src/agent/` |
| agent integration tests | `crates/server/tests/agent_{detect,edit,install,hook}.rs` |

## 2. Transport [T]

- **T1** One HTTP listener serves everything: `GET /ws` (WebSocket upgrade), the
  JSON side-channel of §2.1, and a static-asset fallback for every other path.
- **T2** A client MUST use exactly one WebSocket connection for all of its
  sessions. Concurrency is expressed by channels (§4), never by extra sockets.
- **T3** Scheme follows page origin (`ws:` / `wss:`). TLS is not terminated in
  the daemon; deployment beyond localhost is a network-layer concern.
- **T4** Every application message MUST be a WebSocket **binary** message. Text
  messages are rejected during handshake (T4a) and silently ignored afterwards.
- **T5** Framing (§3) is transport-independent by design. A future raw-TCP/QUIC
  transport reuses §3 unchanged. Any rule that depends on WebSocket semantics
  (Ping/Pong, message boundaries) is stated explicitly and MUST NOT be assumed
  elsewhere.
- **T6** One transport message carries exactly one frame. A frame never spans
  messages; a message never carries two frames. Decoders rely on this to derive
  payload length (F4).
- **T7** Liveness uses WebSocket control frames only. The server sends Ping
  every 30 s and drops a connection from which it has received nothing —
  Pong included — for 90 s. Clients MUST answer Pings (browsers do so
  automatically) and are not required to run their own liveness timer. No
  application-level heartbeat message exists or may be added without §12.

### 2.1 JSON side-channel [T]

- **T8** A small number of routes under `/api/` serve plain JSON over HTTP,
  outside the framing of §3 and outside the connection state machine of §5.
  They exist for data that is **session-independent, low-frequency, and needed
  before (or without) a socket**. Nothing that concerns a session, a channel, or
  a byte stream may live here — that is §6 and §7 territory.
- **T9** Every `/api/` route requires `Authorization: Bearer <token>` with the
  same token as `hello` (H3). Missing or wrong → `401`, empty body, no detail.
  The header, not a query parameter: query strings leak into logs and history,
  and requiring a header means the request must come from script, which keeps
  cross-site `<form>`/`<img>` requests out.
- **T10** The **read-only subset** is stateless and idempotent: `GET` only, safe
  to repeat, no effect on any session. A client MAY call one at any time,
  including before the WebSocket handshake.
- **T10a** A **mutating subset** exists under `/api/agents/`. These routes are
  `POST`, change state outside octoterm (they edit another program's
  configuration file), and are gated by server configuration
  (`agents.install_enabled`, default off) — a disabled deployment answers `403`.
  They are still session-independent: nothing here concerns a session, a channel
  or a byte stream. Rationale for keeping them off the control channel: a new
  client→server control message is breaking (X3) and would force a proto bump
  for a request that is low-frequency and expressible over HTTP.
- **T11** Failure is reported by HTTP status, not by a `ServerMsg`. `error`
  (§9) belongs to the socket and never appears here.
- **T12** A client MUST tolerate any `/api/` route being absent (`404`) or
  failing: an older server has no such route. Degrade, do not block. Because of
  this rule, adding a route is compatible in both directions and does **not**
  bump `proto` (X4-equivalent; the routes are not versioned by `proto` at all).
- **T13** Current routes:

| route | reply | notes |
| --- | --- | --- |
| `GET /api/launchers` | `{ "launchers": [Launcher] }` | see §2.2 |
| `GET /api/agents` | `{ "agents": [AgentStatus] }` | which agents are installed on the host, and whether octoterm's integration is in place |
| `GET /api/agents/{id}/plan` | `{ "install": [...], "uninstall": [...] }` | dry run: what editing the agent's config would do |
| `POST /api/agents/{id}/install` | `{ "changed": bool, "files": [...] }` | T10a; `403` when disabled |
| `POST /api/agents/{id}/uninstall` | same | T10a |
| `GET /api/agents/sessions` | `{ "sessions": [AgentSession] }` | full snapshot; a client re-fetches this after every reconnect (A5) |

### 2.2 `GET /api/launchers` [T]

- **T14** Returns the candidate commands a client may offer under "new
  session". Entries are discovered server-side by *providers*: the built-in
  default shell, the operator's own `[[launcher]]` entries in `config.toml`, and
  **read-only** scans of other terminals' configuration already present on the
  host (iTerm2, Windows Terminal). octoterm never writes those files.
- **T15** `Launcher` shape:

```
Launcher { id:str, provider:str, name:str, detail:str, command:[str], cwd:str? }
```

  - `id` — `"<provider>:<provider-local id>"`, stable across server restarts so
    a client may remember a previous choice. Unique within one reply.
  - `name` — display name, from the source profile. **Not unique**, never a key.
  - `detail` — one-line human-readable command preview. Presentation only; a
    client MUST NOT parse it.
  - `command` — argv, non-empty, directly spawnable.
  - `cwd` — working directory, or `null`.
- **T16** The list is ordered for display and MUST be presented in the order
  given: the built-in default shell first (it is the only entry guaranteed to
  exist and to work), then operator-defined entries, then scanned ones.
- **T17** Discovery runs on every request; there is no cache and no
  invalidation event. A profile added in iTerm2 shows up on the next request.
- **T18** A failing provider is skipped, not fatal: a corrupt third-party
  config removes that provider's entries and nothing else. The reply therefore
  always contains at least the built-in entry, and `200` does **not** mean every
  provider succeeded. Providers do not report their failures to the client.
- **T19** A client MUST NOT assume any particular provider exists; `provider`
  is an opaque string used for grouping and labelling. Unknown values are
  displayed verbatim.
- **T20** `command` and `cwd` are passed back **verbatim** in `new-session`
  (§6.1). The server does not remember which launcher was chosen and does not
  resolve ids at spawn time; `id` is for the client's own bookkeeping.

## 3. Frame format [F]

- **F1** Layout:

  ```
  offset  size  field
  0       4     channel  u32, little-endian
  4       1     flags    u8
  5       N     payload  opaque
  ```

- **F2** Minimum length is 5 bytes. Shorter input: rejected during handshake,
  silently dropped afterwards.
- **F3** `flags` is reserved. Senders MUST write 0; receivers MUST ignore it in
  v1. Future use MUST be per-bit additive and MUST be gated by X-rules.
- **F4** Payload has no length prefix; it runs to the end of the transport
  message (T6).
- **F5** No compression, encryption, checksum or fragmentation at this layer.
- **F6** Example — `hello` on the control channel:

  ```
  00 00 00 00 | 00 | 7b 22 74 79 70 65 22 3a 22 68 65 6c 6c 6f 22 ...
  channel=0   |flags| {"type":"hello","token":"…","proto":1}
  ```

## 4. Channels [CH]

- **CH1** Channel `0` is the control channel. Payload is UTF-8 JSON, one
  control message per frame.
- **CH2** Channel `!= 0` is an attached session. Payload is a raw VT byte
  stream, bidirectional: server→client is pty output, client→server is input.
- **CH3** Channel ids are allocated by the client and scoped to one connection.
  They are not global and not required to be stable across reconnects, though a
  client SHOULD keep them stable to simplify resume bookkeeping.
- **CH4** `attach` on channel 0, or on a channel already attached on this
  connection, is refused with `error{channel}` — the server never reassigns.
- **CH5** Multiple connections MAY attach the same session id concurrently on
  their own channels. A session is a shared broadcast resource, never owned by
  a connection.
- **CH6** Detach or connection loss never terminates a session. Only
  `kill-session` or child exit does.
- **CH7** A data frame on a channel that is not attached on this connection is
  dropped and answered with `error{channel}`.

## 5. Handshake and authentication [H]

- **H1** The first frame after the socket opens MUST be control `hello`.
- **H2** On match the server replies `hello-ok{proto}`; the connection is then
  authenticated. No other message is processed before this point.
- **H3** Token or proto mismatch → `error` then close. Token comparison is
  exact string equality.
- **H4** Non-binary first message → `error` then close.
- **H5** No first message within 5 s → `error` then close.
- **H6** `hello` after authentication → `error`; the connection stays open.
- **H7** The token travels in-band because browsers cannot set WebSocket
  headers. It reaches the page via URL fragment (`#token=…`) and MUST NOT be
  placed in the query string or path — fragments are not sent to servers,
  proxies or logs.
- **H8** Static assets are served unauthenticated by design; they contain no
  secret. Only `/ws` is guarded.

## 6. Control messages [C]

- **C1** Encoding: a single JSON object per frame, internally tagged; `"type"`
  holds the kebab-case variant name. UTF-8, no trailing framing.
- **C2** Field names are snake_case (`last_seq`, `created_at`); enum values are
  kebab-case.
- **C3** Decoding is **lenient about unknown fields** (ignored on both sides)
  and **strict about unknown types** (server rejects; clients ignore — see X2).
- **C4** An absent optional field and an explicit `null` are equivalent on
  decode; both MUST be accepted. The Rust encoder emits explicit `null` except
  for `error.channel`, which is omitted when absent.
- **C5** There is no request id and no reply correlation. Replies that need
  attribution carry a natural key instead (`preview-data.id`,
  `attached.channel`, `error.channel`). A client issuing concurrent same-type
  requests without such a key cannot attribute the responses.
- **C6** Mutations (`new-session`, `kill-session`, `rename-session`) have no
  success reply. Confirmation arrives as a `session-event` broadcast to every
  connection; failure arrives as a targeted `error`.

### 6.1 Client → server

| type | fields | legal state | effect / reply |
| --- | --- | --- | --- |
| `hello` | `token:str`, `proto:u32` | pre-auth only | → `hello-ok`, else `error`+close (H3–H6) |
| `list-sessions` | — | auth | → `sessions` |
| `new-session` | `name:str?`, `command:[str]?`, `cwd:str?` | auth | spawns pty; → `session-event{created}` broadcast (C6) |
| `kill-session` | `id:u64` | auth | kills child; → `session-event{closed}` when reaped |
| `rename-session` | `id:u64`, `name:str` | auth | → `session-event{renamed}` broadcast |
| `preview` | `id:u64` | auth | → `preview-data{id}` |
| `attach` | `id:u64`, `channel:u32`, `last_seq:u64?`, `cols:u16`, `rows:u16` | auth | registers this attachment's desired size (G2), then → `attached` + recovery burst (§8) |
| `detach` | `channel:u32` | attached | stops the pump, drops the desired size (G8); session keeps running (CH6) |
| `resize` | `channel:u32`, `cols:u16`, `rows:u16` | attached | updates this attachment's desired size (G2) |

`name: null` → server-generated default (U5). `command: null` → the built-in
default shell, which is by construction the first entry of `GET /api/launchers`
(`$SHELL` or `/bin/sh` on unix, `powershell.exe` or `%ComSpec%` on Windows).
`cwd: null`, or a path that is not an existing directory, → the server's default
working directory (`$HOME` and friends); a bad `cwd` is a warning in the log, not
a spawn failure — a profile may have been written on another machine. Sessions
always get `TERM=xterm-256color`, `COLORTERM=truecolor`.

`command` and `cwd` come straight from the client. This grants no privilege the
client did not already have: `command` has always been arbitrary, and everything
runs as the user who started the daemon. The bearer token is the only boundary
(U6).

### 6.2 Server → client

| type | fields | when |
| --- | --- | --- |
| `hello-ok` | `proto:u32` | handshake accepted |
| `error` | `message:str`, `channel:u32?` | §9 |
| `sessions` | `sessions:[SessionInfo]` | reply to `list-sessions`, sorted by id |
| `session-event` | `event:SessionEventKind`, `session:SessionInfo` | broadcast on create/rename/close |
| `preview-data` | `id:u64`, `data:str` | reply to `preview`; `data` = base64 of an ANSI repaint (D6) |
| `attached` | `channel:u32`, `seq:u64`, `mode:AttachMode` | first reply to `attach` |
| `resized` | `channel:u32`, `cols:u16`, `rows:u16` | authoritative geometry of the session, on every attached channel (G5, G6) |
| `resync-begin` | `channel:u32` | opens a resync burst (S5) |
| `resync-end` | `channel:u32`, `seq:u64` | closes a resync burst; authoritative anchor (S7b) |
| `session-exited` | `channel:u32`, `id:u64` | child exited; the channel's stream is over (S11) |
| `agent-event` | see §15 | a coding agent inside a hosted session changed state |

### 6.3 Shared types

```
SessionInfo      { id:u64, name:str, cols:u16, rows:u16, created_at:u64 }  # created_at = unix seconds
AttachMode       "replay" | "resync"
SessionEventKind "created" | "renamed" | "closed"
```

## 7. Data plane [D]

- **D1** Server→client on channel `c`: pty output bytes, verbatim, no
  re-encoding.
- **D2** Client→server on channel `c`: input bytes, written verbatim to the pty
  master.
- **D3** The server MAY coalesce many pty reads into one frame, up to 64 KiB of
  payload. Frame boundaries are **meaningless**: a frame MAY split a UTF-8
  sequence or an escape sequence. A parser fed frames in order MUST behave
  identically to one fed the concatenated stream. Clients MUST NOT buffer,
  reorder or re-chunk on assumptions about boundaries.
- **D4** Data frames carry no sequence number. Accounting is §8.
- **D5** There is no credit or ack mechanism. Flow control is TCP/WebSocket
  backpressure plus lossy drop (S6).
- **D6** `preview-data` is the sole place VT bytes travel over the control
  channel, base64-encoded, because a preview has no channel. It is bounded by
  one screen. New messages MUST NOT carry bulk VT bytes over channel 0 (R4).

## 8. Session state, resume and seq [S]

- **S1** Per session the server keeps an authoritative `alacritty_terminal`
  grid plus a ring buffer of raw output bytes (default 1 MiB) with a monotonic
  byte counter `seq`.
- **S2** `seq` counts pty output bytes ever produced, from 0, and never resets
  while the session lives. Bytes evicted from the ring keep their seq; the ring
  exposes `[start_seq, end_seq]`.
- **S3** `attach` with `last_seq: null` always yields **resync**.
- **S4** `attach` with `last_seq: n`: if `n ∈ [start_seq, end_seq]` →
  **replay** — `attached{mode:"replay", seq:end_seq}` followed by one data
  frame with bytes `[n, end_seq)` (omitted when empty). Otherwise → **resync**.
- **S5** A resync burst on channel `c` is exactly, in order:
  `resync-begin{c}` → one data frame on `c` carrying a synthesized ANSI repaint
  → `resync-end{c, seq}`. The client MUST reset its local emulator on
  `resync-begin`. Repaint bytes are synthesized and are **not** part of the seq
  stream. Every burst is immediately preceded by a `resized` (G6), which is not
  part of the burst.
- **S6** Lossy backpressure: when a connection's broadcast queue lags, the
  server discards the bytes that connection missed and issues a fresh resync
  (S5) mid-stream. Clients MUST accept a resync at any time, not only at attach.
- **S7** **seq bookkeeping invariant (client).** `last_seq` may be anchored
  from exactly two sources:
  - **S7a** the `last_seq` the client itself sent in an `attach` that was
    answered with `mode: "replay"` — the replayed bytes continue from that
    point;
  - **S7b** `resync-end.seq`.

  After anchoring, `last_seq += payload.length` for every data frame received
  on that channel. Before anchoring, data frames MUST NOT be counted.
- **S8** `attached.seq` MUST NOT be used as an anchor. In replay mode it is
  informational (where the replay ends) and using it double-counts against S7a.
  In resync mode it is meaningless (`0`); the authority is `resync-end.seq`.
- **S9** On reconnect the client re-sends `attach` for every live channel with
  its tracked `last_seq`, after `hello-ok` (H2) and never before.
- **S10** A resync restores content, cursor position and visibility, and the
  common modes: app cursor keys (`?1`), app keypad, bracketed paste (`?2004`),
  mouse reporting (`?1000/1002/1003/1005/1006`). It does **not** restore the
  alt screen, DECSTBM scroll regions, or scrollback. Accepted lossy tradeoff;
  full-screen apps may need `Ctrl-L` after a rough reconnect.
- **S11** `session-exited{channel,id}` ends that channel's stream; the server
  stops the pump. The client MUST treat the channel as dead and stop sending on
  it. A `session-event{closed}` for the same id is broadcast independently and
  its ordering relative to `session-exited` is NOT guaranteed — clients MUST
  tolerate either order and MUST treat both as idempotent.

### 8.1 Geometry [G]

- **G1** A session has exactly one geometry: one pty size, one grid. Every
  attachment of that session receives the same byte stream (CH5, D1), so the
  server cannot crop or reflow per client. Geometry is session state, not
  connection state.
- **G2** `attach{cols,rows}` and `resize{cols,rows}` are **requests, not
  commands**: they register (attach) or update (resize) that attachment's
  desired size. The server merges the desired sizes of all live attachments
  into the authoritative geometry.
- **G3** The merge policy is server configuration, not part of the wire
  (`window-size`: `smallest` — the default, per-dimension minimum, nobody sees
  a truncated screen — or `largest`, or `latest`). Clients MUST NOT assume the
  geometry they requested was adopted, and MUST NOT infer the policy.
- **G4** The merged value is clamped to a floor of 20×5. A session with no
  attachments keeps its current geometry unchanged — detaching the last client
  MUST NOT reflow the application.
- **G5** When the authoritative geometry changes, the server sends
  `resized{channel,cols,rows}` on **every** attached channel of that session,
  across all connections, including the attachment that caused the change. No
  `resized` is sent when a request leaves the merged value unchanged, so a
  client MUST NOT wait for one as an acknowledgement of its `resize`.
- **G6** Exactly one `resized` precedes the screen bytes in every recovery
  path: before the replayed data frame in replay mode, and immediately before
  `resync-begin` in every resync (attach-time or mid-stream, S5/S6). A client
  therefore always knows the geometry a repaint was rendered for.
- **G7** Clients MUST render at the geometry last announced by `resized`, and
  MUST NOT resize their local emulator on their own — doing so misaligns
  wrapping against a byte stream the server produced for a different width.
  What to do with left-over space (letterbox, scale, scroll) is the client's
  business (U7).
- **G8** Detach, connection loss, or pump shutdown drops that attachment's
  desired size and re-merges. Note that a dead connection is only noticed after
  the liveness timeout (T7), so a vanished client can hold geometry hostage for
  up to 90 s.

## 9. Errors [E]

- **E1** Shape: `{"type":"error","message":str,"channel":u32?}`.
- **E2** `channel` present → the error is scoped to that channel's operation
  (attach/detach/resize/input), so the client can close just that terminal.
  Absent → connection- or session-scoped.
- **E3** Errors are advisory and non-fatal, except handshake errors (H3–H5),
  which are always followed by a close.
- **E4** `message` strings are human- and log-facing and are **not stable**.
  Clients MUST NOT branch on their text.
- **E5** Corollary: if a client needs to branch on a failure, the protocol needs
  a machine-readable code — propose one via §12 rather than parsing strings.
- **E6** A client MUST stop reconnecting when the socket closes after an error
  that arrived before `hello-ok`: that is an auth failure, and retrying only
  repeats the rejection.

Current corpus (reference only, see E4):

| message | trigger | scope |
| --- | --- | --- |
| `hello timeout` | no first frame in 5 s | conn, then close |
| `expected binary hello frame` | non-binary first message | conn, then close |
| `bad hello` | not a `hello`, wrong token, or wrong proto | conn, then close |
| `already authenticated` | second `hello` | conn |
| `malformed control message` | channel-0 payload is not a valid `ClientMsg` | conn |
| `no such session` | `kill`/`rename`/`preview` on unknown id | conn |
| `no such session` | `attach` on unknown id | channel |
| `channel unavailable` | `attach` on channel 0 or an in-use channel | channel |
| `no such channel` | `detach`/`resize`/data on an unattached channel | channel |
| `spawn failed: …` | pty spawn failed | conn |
| `session input failed: …` | pty write failed | channel |
| `resize failed: …` | pty resize failed | channel |

## 10. Limits and timers [L]

| constant | value | source |
| --- | --- | --- |
| frame header | 5 bytes | `frame.rs` |
| max data-frame payload | 64 KiB | `conn.rs COALESCE_MAX` |
| per-session ring buffer | 1 MiB | `main.rs SessionManager::new(1 << 20)` |
| per-session broadcast queue | 256 messages | `pty.rs BROADCAST_CAP` |
| per-connection outbound queue | 64 messages | `conn.rs` |
| pty read chunk | 8 KiB | `pty.rs` |
| hello timeout | 5 s | `app.rs` |
| server Ping interval | 30 s | `conn.rs KEEPALIVE_INTERVAL` |
| read timeout (liveness) | 90 s | `conn.rs READ_TIMEOUT` |
| default session geometry | 80×24 until first attach | `manager.rs` |
| geometry floor | 20×5 | `pty.rs MIN_COLS/MIN_ROWS` |
| default geometry merge policy | `smallest` | `config.rs WindowSize` |
| launcher entries per provider | 100 | `launcher/mod.rs PER_PROVIDER_CAP` |
| `agent-event` payload | 4 KiB | §15 A4 |
| agent hook request body | 512 KiB | `app.rs` route layer |
| reference reconnect backoff | 250 ms doubling, cap 10 s | `client-core`, `client.ts` |

## 11. Compatibility and versioning [X]

- **X1** `proto` is a single u32 compared for exact equality at handshake.
  There is no range negotiation and no capability list.
- **X2** The two directions are **not symmetric**:
  - server→client: a new message *type* is backward compatible — clients ignore
    unknown `type` values;
  - client→server: a new message *type* is **not** — the server fails to
    deserialize and answers `error{"malformed control message"}`, dropping it.
- **X3** Therefore adding a client→server message type is a breaking change and
  REQUIRES a proto bump. No negotiation mechanism exists to soften this; adding
  one is itself a protocol change subject to §12.
- **X4** Adding an **optional** field to an existing message is compatible in
  both directions: unknown fields are ignored, and a missing `Option<T>`
  decodes to `None` (verified behaviour, C4). The field MUST be `Option<T>` and
  its absence MUST be safe.
- **X5** Breaking, REQUIRES a proto bump: changing a field's type or meaning,
  removing or renaming a field, changing an enum's kebab spelling, changing the
  frame layout, or changing `flags` semantics.
- **X6** A proto bump is a hard cutover — every client must be rebuilt and
  redeployed, and every open page breaks. Prefer X4 shapes wherever possible.

## 12. Extension procedure and review checklist [R]

### 12.1 Reuse first

Before proposing anything new, rule out every applicable row:

| what you want | existing mechanism |
| --- | --- |
| bulk or binary per-session stream | a data channel (CH2) |
| the current screen of a session, without attaching | `preview` → `preview-data` |
| the current screen of an attached channel | trigger a resync (S5) |
| session inventory | `list-sessions` → `sessions` |
| a static, session-independent catalogue, wanted before the socket is up | a `GET /api/` route (T8) |
| server-initiated notice about a session's existence or identity | a new `SessionEventKind` on `session-event` |
| server-initiated notice about something *running inside* a session | `agent-event` (§15) — `session-event` is about the session's existence and identity, not its contents |
| per-attachment lifecycle | `attach` / `detach` |
| ask for a different geometry | `resize` — a request, not a command (G2) |
| tell clients the authoritative geometry | `resized` (G5) |
| report a failure | `error`, with `channel` when operation-scoped |
| recover after a gap | `attach{last_seq}` (S4) |

### 12.2 Checklist

Every item MUST be answered in the proposal.

- **R1 Reuse.** State why each applicable row of 12.1 does not fit.
- **R2 Plane.** Control channel only if the payload is small, structured and
  low-frequency. Anything per-keystroke or per-output-burst belongs on a data
  channel.
- **R3 Compatibility.** Classify under X2–X5 and say whether a proto bump is
  required; if it is, justify the cutover.
- **R4 No bulk bytes in JSON.** Base64 over channel 0 is allowed only when no
  channel can carry it and the payload is bounded by roughly one screen (D6).
- **R5 Attribution.** If a reply is expected, name the natural key that ties it
  to the request (C5).
- **R6 Reconnect and repetition.** Define behaviour when sent twice, and when
  in flight across a reconnect. Resume (S7) MUST stay correct.
- **R7 seq impact.** If it injects bytes into a session stream, it MUST either
  go through the ring (counted in seq) or be bracketed like a resync
  (uncounted, ending in a new anchor). There is no third option.
- **R8 State machine.** Which of {pre-auth, authenticated, attached} states is
  it legal in, and what does the server do in the illegal ones?
- **R9 Failure mode.** Which error, and is `channel` set (E2)?
- **R10 Bounds.** State worst-case payload size and message frequency, and add
  the limit to §10 if it introduces one.
- **R11 Artifacts.** Same PR must update: `crates/protocol/src/messages.rs`,
  both fixture files, `clients/web/src/*` when client-facing, tests under
  `crates/server/tests/`, and this document with new rule IDs.
- **R12 Naming.** `type` kebab-case, fields snake_case, enum values kebab-case.
  Verb-object for requests (`kill-session`), state or past tense for events
  (`session-exited`, `attached`).
- **R13 Client neutrality.** Does it require the client to understand a
  server-internal data structure, or does it push window/tab/pane semantics
  into the server? Either one is grounds for rejection: the server hosts
  process + IO + screen state, clients own the UI, and recovery speaks VT, not
  a private format.

## 13. Deliberately unspecified [U]

- **U1** Channel id allocation policy — the client's business; the server only
  enforces CH4.
- **U2** Client-side scrollback retention.
- **U3** Reconnect backoff schedule (reference values in §10).
- **U4** Session geometry before the first attach (currently 80×24).
- **U5** Default session name (currently `octoterm-{id}`).
- **U6** Multi-user identity or authorization beyond one shared bearer token.
- **U7** What the client renders; the server assumes only a VT-compatible sink.
- **U8** Which geometry merge policy a deployment runs, and whether it offers
  more than the three in G3 — server configuration, never negotiated on the
  wire. Clients only ever state a wish and obey `resized`.
- **U9** Which launcher providers a deployment runs, and what a client does with
  the list (menu, palette, remembering the last choice, ignoring it entirely).
  The server states what is available; the client owns the UI (R13).

## 14. Known deviations from the design doc

- **DEV1** `docs/superpowers/specs/2026-08-16-octoterm-design.md` describes
  output coalescing as "~16 ms timer or size threshold". The implementation has
  only the size threshold (non-blocking drain up to 64 KiB in `pump_output`).
  Conformant with D3; the design doc's wording is stale.
- **DEV2** The same doc lists "credit 背压" among the tests. No credit
  mechanism exists or is planned — backpressure is D5 + S6.

## 15. Agent integration [A]

- **A1** `agent-event` is broadcast to every authenticated connection whenever a
  coding agent running inside a hosted session changes state. Shape:

  ```
  agent-event {
      agent_id:str, agent_session_id:str, session:u64?,
      state:AgentState, pending:str?, detail:str?
  }
  AgentState  "idle" | "thinking" | "working" | "waiting" | "done" | "error"
  ```

- **A2** It is a **server→client** addition and therefore compatible in both
  directions (X2): it does **not** bump `proto`. There is deliberately no
  client→server counterpart — answering a pending request is
  `POST /api/agents/answer`, because a new client→server type is breaking (X3)
  and the request is low-frequency.
- **A3** `session` is the hosted session the agent belongs to. It is optional
  only so that supporting agents outside octoterm later does not require a bump
  (X4); the current server always sets it.
- **A4** `pending` is non-null exactly when the agent is blocked waiting for a
  human answer. Its value is the natural key (C5) a client passes back to
  `POST /api/agents/answer`. `detail` is a one-line human-readable string;
  clients display it and MUST NOT parse it. Whole message ≤ 4 KiB (§10).
- **A5** No incremental reconciliation. A client that (re)connects MUST fetch
  `GET /api/agents/sessions` for the full snapshot; `agent-event` only carries
  deltas afterwards. Events missed while disconnected are recovered by that
  fetch, not by replay (R6).
- **A6** `state` is a **closed** enum. Adding a value is breaking for strict
  decoders and MUST go through §12.
- **A7** `agent-event` carries no VT bytes and injects nothing into any session
  stream; it has no effect on `seq` (R7). Taking over an agent's prompt by
  typing into it is ordinary session input on a data channel (§7) — indis-
  tinguishable from a human at the keyboard, and counted in `seq` like any other
  output it produces.
- **A8** The server↔agent side of this feature (how hooks are installed into a
  third-party agent's configuration, and the `/hook/...` ingress they call) is
  **not part of this document**: it is not on the wire between a client and
  octoterm. It is specified in
  `docs/superpowers/specs/2026-08-18-octoterm-agent-integration-design.md`.
  Clients neither see nor depend on it.
