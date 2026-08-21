# octoterm iOS 客户端与 Cloud 协同需求草案

日期:2026-08-20  
状态:需求草案,等待 Review  
性质:产品与系统需求输入,**不是实现 spec**

## 这份文档解决什么

本文把当前关于 iOS 客户端、多个 octoterm server、原生终端渲染、coding agent
状态与远程反馈、后台通知以及未来 Cloud 协同的讨论落成一份可评审的需求基线。

Review 完成后,再基于这里被确认的范围建立正式设计 spec、协议变更和实现计划。
本文中的「建议」与「待决」不能被实现者当作已经批准的设计。

## 已确认的方向

1. 增加一个原生 iOS 客户端,以 iPhone 使用为首要场景。
2. iOS 客户端首先提供连接管理,可保存并连接多个 octoterm server。
3. 进入某个 server 后,会话能力参考 `clients/web`,但界面遵循移动端原生交互,
   不要求逐像素复刻 Web UI。
4. 终端希望以 `libghostty` 获得高性能、正确的 VT 处理与原生 GPU 渲染效果。
5. 客户端必须展示托管 session 中 coding agent 的状态。
6. 用户必须能从移动端对 agent 发出反馈,包括结构化授权/拒绝/选择题,以及必要时
   进入终端继续输入。
7. Agent 等待用户时需要后台通知;不能把「App 前台保持 WebSocket」当作通知方案。
8. 项目接受未来引入真正的 Octoterm Cloud。server 可以主动建立到 Cloud 的长连接,
   经它发送事件、接收反馈与控制信息。
9. server 继续保持终端客户端中立。iOS、Web、未来 Android 与 Cloud 都不能把自己的
   窗口、页面或导航语义塞进本地终端协议。

## 仍待 Review 的关键裁决

按对架构的影响从高到低排序:

1. Cloud 首期只承载控制面与 Agent 事件,还是同时中继交互式终端数据。
2. Cloud 是否必须对终端数据、工具入参和反馈内容做端到端加密,使 Cloud 只能看到
   路由元数据而不能读取内容。
3. server 与手机如何建立所有权:Octoterm 账号、一次性配对码,还是两者并存。
4. 锁屏通知只负责提醒并唤起 App,还是允许在通知上直接批准/拒绝。
5. 首个正式版本的范围是 iPhone only,还是同一版本同时优化 iPad 分屏与硬件键盘。
6. 首发渠道是 TestFlight/自用签名,还是直接以 App Store 审核要求为约束。
7. Cloud 是项目运营的托管服务、自托管服务,还是两种部署方式都要支持。

## 背景与现有基础

octoterm server 已经承担 PTY 生命周期、服务端权威 grid、断线恢复与会话管理。客户端
通过一条 WebSocket 连接,使用 channel 复用多个 session。非零 channel 是原始 VT
字节流,channel 0 是 JSON 控制消息。

现有协议已经提供:

- session 列表、新建、改名、关闭;
- 不 attach 即获取当前屏幕重绘序列的 preview;
- attach、detach、resize;
- ring buffer replay 与超窗后的 resync;
- server 权威终端尺寸;
- agent 状态增量事件;
- agent session 与 pending request 全量快照;
- allow、deny 和 `AskUserQuestion` 选择题回答;
- 通过普通终端输入对 agent 进行自由文本反馈。

因此,前台直连版 iOS 客户端原则上不需要修改本地 server 协议。Cloud 与推送能力是
新增的系统平面,不应为图省事破坏现有客户端中立协议。

## 产品目标

### G1. 多主机入口

用户在一台 iPhone 上管理自己分布在桌面机、笔记本、家用服务器和云主机上的多个
octoterm server,不需要记忆每台机器的 URL 与 token。

### G2. 随时接管活着的终端

用户可以查看各 server 的 session,进入任意 session,在网络切换和短暂断线后恢复到
可用画面,而不重启远端 shell 或 agent。

### G3. Agent Inbox

用户不必逐台 server、逐个 session 寻找 agent。客户端提供跨 server 聚合的 Agent
Inbox,优先展示正在等待人的请求。

### G4. 后台可达

App 被挂起或手机锁屏时,Cloud 仍能从 server 收到 Agent 等待事件,并通过 APNs 提醒
用户。用户打开通知后能看到完整上下文并作出反馈。

### G5. 不盲签

任何允许执行工具、命令或文件修改的入口,都必须让用户在作出决定前看到足够的原始
上下文。不得把截断后的命令或只有「Allow」按钮的通知当作完整审批界面。

### G6. 保持 server 克制

多 server 导航、标签页、通知展示、iOS 场景恢复等属于客户端。Cloud 只增加远程可达
与路由能力,不把 octoterm server 变成通用 IDE、文件管理器或 agent transcript 服务。

## 非目标

以下内容不进入首轮需求:

- 文件浏览、上传下载或远程编辑器;
- 多用户共享同一个 terminal session;
- agent transcript、token 用量、成本统计;
- 在 server 内引入 iOS 页面、窗口、分栏或导航概念;
- 用 Cloud 替代本地 server 对 PTY 与终端 grid 的所有权;
- 后台持续运行 iOS WebSocket 来规避 APNs;
- 在没有完整上下文时由系统自动批准 agent 请求;
- 首版同时交付 Android 客户端。

## 领域术语

**Local Server** — 当前运行在用户主机上的 `octoterm-server`,拥有 PTY、session、grid
与 agent hook 状态。

**Cloud** — 可公网访问的协调系统。它接收 Local Server 的主动连接,登记设备,
转发状态与反馈,并作为 APNs provider。是否同时中继终端数据尚待决定。

**Server Profile** — iOS 中保存的一台 Local Server 的显示名、连接地址、信任策略、
Cloud 绑定和凭据引用。token 本身不放普通偏好存储。

**Hosted Session** — Local Server 托管的 PTY 会话,由 octoterm session id 标识。

**Agent Session** — coding agent 自己的逻辑会话,可以关联到一个 Hosted Session。
同一个 Hosted Session 中可能先后或同时出现多个 Agent Session。

**Pending Request** — agent 当前阻塞等待人的一次请求,拥有全局不可猜的 id 与明确的
过期时间。允许、拒绝和选择题回答都以它作为自然键。

**Direct Path** — iOS 直接连接 Local Server 的 HTTP/WebSocket 路径。

**Cloud Path** — iOS 与 Local Server 经 Cloud 交换事件、反馈或数据的路径。

## 信息架构

建议的顶层导航:

```text
Octoterm
├── Servers
│   ├── Server 列表
│   └── Server 详情
│       ├── Session 列表
│       └── Terminal
├── Agents
│   ├── Waiting
│   └── Active / Recent
└── Settings
```

`Servers` 回答「去哪台机器」。`Agents` 回答「哪里正在等我」。二者必须共享同一份
server/session/agent 状态,不能各自维护一套最终会漂移的缓存。

## 功能需求

以下使用 MUST / SHOULD / MAY 表示需求强度。编号用于 Review 与后续 spec 引用,
在需求确认后保持稳定。

### 1. Server Profile 与连接管理

- **IOS-CONN-01 MUST** 支持新增、编辑、删除和排序多个 Server Profile。
- **IOS-CONN-02 MUST** 每个 profile 至少包含显示名、Local Server URL、凭据引用与
  TLS/本地网络信任状态。
- **IOS-CONN-03 MUST** bearer token、Cloud refresh credential 与配对私钥存入 iOS
  Keychain,不得写入 `UserDefaults`、日志或 crash breadcrumb。
- **IOS-CONN-04 MUST** 区分 connecting、online、reconnecting、offline、auth failed、
  incompatible 和 unpaired 状态。鉴权失败不得无限重试。
- **IOS-CONN-05 MUST** 前台时允许同时观察多台 server 的 session 与 agent 状态;
  一台 server 失败不得阻塞其他 server。
- **IOS-CONN-06 SHOULD** 支持粘贴 server 启动时生成的完整访问 URL,自动拆出地址与
  token,并在保存前让用户确认。
- **IOS-CONN-07 SHOULD** 后续支持二维码/一次性配对码,避免手工输入长 token。
- **IOS-CONN-08 MUST** 删除 profile 前说明会删除哪些本地凭据和 Cloud 绑定;删除必须
  可与「仅从本机移除」和「撤销该设备授权」区分。

### 2. Server 与 Session 管理

- **IOS-SESSION-01 MUST** 展示一台 server 的 session 名称、id、尺寸、创建时间和
  关联 agent 的最高优先级状态。
- **IOS-SESSION-02 MUST** 支持新建、改名、关闭 session。
- **IOS-SESSION-03 MUST** 新建 session 菜单使用 server 的 `/api/launchers`,保持
  server 返回顺序,并容忍旧 server 没有该路由。
- **IOS-SESSION-04 MUST** session 生命周期通过现有全量列表与 `session-event` 归并;
  事件重复或乱序不得导致崩溃。
- **IOS-SESSION-05 SHOULD** 提供 session 屏幕预览,但允许在首个可用版本中延后。
  实现时只为可见条目懒加载,不得为整个列表常驻大量 GPU surface。
- **IOS-SESSION-06 MUST** 从 Terminal 返回列表时显式 detach,避免不可见的手机尺寸
  长时间参与 server 权威尺寸归并。

### 3. 原生终端

- **IOS-TERM-01 MUST** 终端消费和产生原始字节,不得把 PTY 数据转成 JSON 或字符串
  后再传输。
- **IOS-TERM-02 MUST** 实现协议规定的 replay、resync 和 `last_seq` 记账不变式。
- **IOS-TERM-03 MUST** 收到 `resync-begin` 时重置本地 emulator;合成重绘字节不计入
  seq;收到 `resync-end` 后使用其 seq 重新锚定。
- **IOS-TERM-04 MUST** 服从 server 的 `resized{cols,rows}` 权威尺寸。手机视口只提出
  desired size,不能自行改变 emulator 网格后假装 server 已接受。
- **IOS-TERM-05 MUST** 软件键盘、安全区、横竖屏与字体变化能产生稳定的 desired
  geometry,并抑制无意义的 resize 风暴。
- **IOS-TERM-06 MUST** 支持中英文输入法、组合输入、硬件键盘、复制粘贴和基本文本
  选择。
- **IOS-TERM-07 MUST** 提供适合手机的 Esc、Ctrl、Alt、Tab 与方向键工具栏。
- **IOS-TERM-08 SHOULD** 支持 OSC 8 链接与安全的外部 URL 打开确认。
- **IOS-TERM-09 SHOULD** 以前台交互时视觉目标为设备刷新率下平滑滚动与低延迟输入;
  精确性能阈值在 Ghostty 技术 spike 后写入 spec。
- **IOS-TERM-10 MUST** Ghostty 崩溃、初始化失败或 surface 创建失败时给出可恢复错误,
  不能让整个连接目录不可用。

### 4. Agent 状态与反馈

- **IOS-AGENT-01 MUST** 识别现有六态:`idle | thinking | working | waiting | done |
  error`。
- **IOS-AGENT-02 MUST** App 启动、server 重连和 Cloud 状态断点恢复后拉取全量 agent
  snapshot,不能只依赖增量事件。
- **IOS-AGENT-03 MUST** 提供跨 server 的 Agent Inbox,`waiting + pending` 排在最高
  优先级。
- **IOS-AGENT-04 MUST** 从 Agent 条目能定位到 server、Hosted Session 与 Agent
  Session,并一键打开对应 Terminal。
- **IOS-AGENT-05 MUST** 回答 pending 前获取完整 Pending Request,展示 agent、server、
  session、tool name、未截断的关键输入和剩余时间。
- **IOS-AGENT-06 MUST** 支持 allow、deny 和 `AskUserQuestion` 单选/多选回答。
- **IOS-AGENT-07 MUST** 区分回答成功、请求已过期/消失、其他设备已经回答和网络失败。
- **IOS-AGENT-08 MUST** 多设备同时回答时只有一次生效;重复提交必须得到明确且幂等的
  结果,不得把 `already answered` 当成未知失败。
- **IOS-AGENT-09 MUST** 对无法结构化呈现的问题回落到「打开 Terminal」,不得只渲染
  一部分选项。
- **IOS-AGENT-10 SHOULD** 支持用户附加 allow/deny message。
- **IOS-AGENT-11 MAY** 在前台提供直接向当前 terminal 写入自由文本的快捷反馈入口;
  它本质上仍是普通 PTY 输入,不能伪装成结构化 agent 回答。

### 5. Cloud 与后台通知

- **CLOUD-01 MUST** Local Server 主动建立到 Cloud 的出站加密长连接。部署不得要求
  Cloud 主动穿透用户路由器连接 Local Server。
- **CLOUD-02 MUST** server 的 Cloud 凭据与本地客户端 bearer token、agent hook token
  相互独立,可以单独轮换和撤销。
- **CLOUD-03 MUST** Cloud 能将 server、session 和 agent 的必要状态路由给已授权设备,
  并在断线后用全量快照恢复一致性。
- **CLOUD-04 MUST** Agent 进入可远程处理的 waiting 状态时,Cloud 通过 APNs 发送可见
  通知;不能依赖 silent push 的及时或必达。
- **CLOUD-05 MUST** APNs payload 只包含通知路由所需的最小标识和非敏感摘要。token、
  完整命令、文件内容、tool_input 与 terminal 字节不得进入明文 push payload。
- **CLOUD-06 MUST** 用户点开通知后,App 获取当前权威 Pending Request;不能直接使用
  可能已经过期的 push 内容作决定。
- **CLOUD-07 MUST** iOS 发出的反馈即使暂时无法到达 Local Server,也必须有明确状态:
  queued、delivered、expired、already answered 或 failed。不能展示虚假的成功。
- **CLOUD-08 MUST** Pending Request 过期后,Cloud 停止投递并撤销或更新尚未处理的
  客户端提醒。
- **CLOUD-09 MUST** 一台 server 离线时,Cloud 能展示 last seen 与数据新鲜度,但不能把
  历史状态伪装成当前状态。
- **CLOUD-10 MUST** Cloud 故障不得破坏同一网络内的 Direct Path。本地直连继续工作,
  Cloud 能力应可配置并默认不改变现有 server 行为。
- **CLOUD-11 MUST** Cloud 发到 Local Server 的每个有副作用命令都带唯一 command id,
  支持安全重试与去重。
- **CLOUD-12 SHOULD** 同一事件的多设备通知可合并,但不能把不同 Pending Request 合并
  成一个无法归因的提醒。
- **CLOUD-13 待决** Cloud 是否中继 Terminal data plane。如果不做,Cloud 只保证 Agent
  状态与结构化反馈;打开完整 Terminal 仍要求 Direct Path 可达。如果做,应复用现有
  frame/channel 语义而不是另造字符串终端协议。
- **CLOUD-14 待决** Cloud 是否保存离线事件。建议只保存短生命周期、可过期、端到端
  加密的事件信封,不保存 terminal scrollback 或 agent transcript。

### 6. 通知交互与安全体验

- **IOS-NOTIFY-01 MUST** 通知至少包含用户可理解的 server 别名、session/agent 标识和
  「需要处理」状态,同时遵守锁屏隐私设置。
- **IOS-NOTIFY-02 MUST** 默认动作是「查看详情」,进入经过设备解锁保护的审批界面。
- **IOS-NOTIFY-03 MUST** 未展示完整工具输入前不得提供可直接生效的 Allow。
- **IOS-NOTIFY-04 待决** 是否允许从通知直接 Deny。即使允许,也必须把过期、已回答与
  server 离线作为正常结果处理。
- **IOS-NOTIFY-05 SHOULD** 用户可按 server 或 agent 配置通知,至少支持全部、仅 waiting、
  静音三档。
- **IOS-NOTIFY-06 SHOULD** 支持 Focus/系统通知权限被关闭时的清晰自检,不能把「APNs
  已接受」等同于「用户一定看到」。

## Cloud 协同模型草图

```text
coding agent
    │ hook
    ▼
Local Server ───── outbound secure connection ─────► Octoterm Cloud
    │                                                    │
    │ owns PTY/grid/session                              ├── device registry
    │                                                    ├── event routing
    │◄──── feedback / command ───────────────────────────┤
    │                                                    └── APNs provider ──► iPhone
    │                                                                          │
    └◄──────────── Direct Path or optional Cloud relay ────────────────────────┘
```

这个模型把所有权留在 Local Server:

- PTY、session 生死、terminal grid 和 pending 的最终状态都以 Local Server 为准;
- Cloud 负责可达性、设备授权、事件路由、短期离线信封和 APNs;
- iOS 不因为收到 push 就假定请求仍有效,必须重新取权威状态;
- Cloud 不应根据过期缓存替用户决定;
- Direct Path 与 Cloud Path 可以并存,客户端按可达性和用户策略选择。

## 数据分类与隐私要求

| 数据 | 敏感度 | Cloud 建议 |
| --- | --- | --- |
| server/device id、连接状态、last seen | 中 | 可路由、最小化保存 |
| session 名称、agent 状态 | 中到高 | 只向已授权设备提供 |
| pending id、过期时间 | 高 | 短期保存、过期删除 |
| tool name、tool input、命令与文件路径 | 很高 | 建议端到端加密,不进 APNs 明文 |
| allow/deny/选择题答案 | 很高 | 鉴权、完整性保护、可审计投递 |
| terminal 输入输出字节 | 极高 | 若中继则必须单独评审 E2EE 与留存策略 |
| bearer/hook/cloud 私钥 | 最高 | 不进入 Cloud 事件或日志 |

Cloud 日志默认不得记录 terminal 字节、tool_input、反馈 message、token 或解密后的敏感
payload。诊断应依赖 opaque id、时间、大小、状态码和链路阶段。

## 失败与竞争语义

### App 不在线

Cloud 发送可见 APNs 通知。App 启动后获取全量 snapshot 与当前 pending 列表,不重放
已经失效的增量事件。

### Local Server 不在线

Cloud 可以短期排队一个尚未过期的结构化反馈,但 UI 必须显示 `queued`,不能显示
`answered`。到 `expires_at` 仍未送达则变成 `expired`。

### 两台手机同时回答

Local Server 是最终裁决者。第一个有效回答成功,其余设备收到 `already answered`,
随后通过 snapshot/event 消除本地 waiting 状态。

### Cloud 不在线

同网可达时 Direct Path 继续工作。后台通知不可用应显式显示为 Cloud degraded,不能
影响现有 Web 客户端或本地 server session。

### iOS 收到通知但请求已过期

打开后显示「请求已经结束」并提供跳转 Terminal 的选项,不重新创建请求,不把旧答案
写入 PTY。

### 网络在 terminal 输入后中断

现有协议没有 client input ack。客户端不得无条件重放可能已经送达的按键,否则可能
重复执行命令。该限制沿用现有 Direct Path;若 Cloud relay 想提供 exactly-once input,
必须另立协议设计,不能在客户端猜测。

## libghostty 集成要求与前置 spike

当前 Ghostty 上游区分两类能力:

- `libghostty-vt`:面向外部使用的 VT 解析与终端状态库,不提供完整的 Ghostty Metal
  绘制界面;
- Ghostty 内部 embedder surface:包含 Apple/Metal 路径,但接口面向 Ghostty 自己的
  macOS App,远端 custom I/O 仍可能需要固定 fork。

因此,正式承诺 iOS Terminal 实现之前先做一个可丢弃的 spike。通过标准:

1. 真机与 simulator 都能构建固定版本的 XCFramework;
2. 能将 server 的任意二进制 VT frame 喂给 surface,包括拆开的 UTF-8 与 escape
   sequence;
3. Ghostty 产生的键盘、IME 和粘贴字节能通过 callback 回送现有 session channel;
4. 能按 `resized` 精确建立 server 权威 `cols × rows`,并在剩余空间 letterbox/scroll,
   而不是改变网格;
5. `resync-begin` 能彻底重置 emulator,随后重绘画面正确;
6. 软键盘展开/收起与旋转不会造成 resize 循环;
7. 连续创建、attach、detach、销毁 surface 不出现崩溃、use-after-free 或明显 GPU
   内存增长;
8. App 进入后台、恢复前台后能安全重建 surface 并通过 resync 恢复;
9. 固定 Ghostty commit、fork 差异、构建脚本与第三方许可证能一起提交仓库;
10. 若 spike 失败,正式 spec 必须重新裁决:维护 custom-I/O fork、使用
    `libghostty-vt + 自有 renderer`,或更换首版 renderer。

## 非功能需求

### 兼容性

- 首版至少兼容当前 `proto_version: 1`。
- 对缺少新增 `/api/` 路由的旧 server 降级,不阻断基本 terminal。
- Swift 协议 codec 使用现有 JSON fixtures 与 Rust/TypeScript 实现交叉验证。
- iOS 最低系统版本在 Ghostty spike 后确定,不得只凭 SwiftUI 偏好提前锁定。

### 性能与能耗

- terminal 数据处理保持字节路径,避免不必要的 UTF-8 String 往返与主线程复制。
- 一台 server 一条前台 WebSocket,session 通过 channel 复用;不得为每个 session 建 socket。
- 非可见 terminal surface 应 detach、暂停或销毁,避免持续 GPU 刷新。
- 后台状态依赖 APNs 与 Cloud snapshot,不滥用后台任务维持轮询。
- 性能阈值由 Ghostty spike 在至少一台较旧 iPhone 和一台当前 iPhone 上测量后写入 spec。

### 可访问性与本地化

- Server、Session、Agent Inbox 与审批 UI 必须支持 VoiceOver 和 Dynamic Type。
- Terminal 字体缩放可以独立于系统正文 Dynamic Type,但必须有可发现的设置。
- 首版文案至少沿用项目现有中文/英文语言方向。
- Agent 状态不能只用颜色表达。

### 可观测性

- 客户端能导出不含凭据和 terminal 内容的诊断日志。
- Local Server 与 Cloud 能通过 opaque event/command id 关联一次通知和反馈链路。
- 需要区分 Local Server 离线、Cloud 断线、APNs 注册失败、通知权限关闭和用户静音。

## 建议的交付阶段

### Phase 0:Ghostty 与协议 spike

只证明 native surface、remote custom I/O、权威 geometry、resync 和生命周期可行。不做
完整产品 UI。

### Phase 1:前台 Direct Client

完成 Server Profile、Keychain、前台多 server 状态、session 管理、单个活动 Terminal、
Agent Inbox 与现有 HTTP/WS 反馈。此阶段不依赖 Cloud,也不修改 Local Server 协议。

### Phase 2:Cloud 控制面与 Push

完成 server 出站连接、配对/账号、设备注册、agent 状态路由、APNs、pending 详情获取、
反馈排队/投递/去重和 Cloud degraded 状态。

### Phase 3:终端 Cloud Path(待裁决)

如果产品要求在 Direct Path 不可达时仍能打开完整 Terminal,增加终端数据中继、流控、
E2EE、配额和滥用防护。否则本阶段可以不做,Cloud 保持更小的控制面。

### Phase 4:体验深化

Session preview、iPad 分屏、多窗口、外接键盘定制、iCloud 同步非秘密 profile、更多
Agent 反馈类型与 Android 共享客户端核心。

## 首轮验收场景

1. 用户保存两台 server,一台在线、一台离线;在线 server 的 session 与 agent 仍正常
   展示,离线 server 明确显示 last seen。
2. 用户进入 session,切换 Wi-Fi/蜂窝网络后重连,画面经 replay 或 resync 恢复,不会
   重复发送最后一个按键。
3. 用户关闭 Terminal 返回列表,手机 attachment 被及时 detach,不继续限制其他客户端
   的 terminal geometry。
4. Agent 在 Server A 请求执行命令;App 前台时 Agent Inbox 实时出现完整审批入口。
5. App 已挂起时同一请求经 Cloud/APNs 产生通知;用户打开后看到权威、未过期、未截断
   的 tool input,然后 Allow 或 Deny。
6. 手机 A 先回答后,手机 B 再回答得到 `already answered`,两端最终都清除 waiting。
7. Local Server 暂时离线时用户提交反馈,界面显示 queued;恢复后只投递一次。若请求先
   过期则显示 expired,绝不补发旧决定。
8. Cloud 故障时,同网手机和 Web 客户端仍可直连 Local Server。
9. 锁屏通知和诊断日志中都找不到 token、完整命令、文件内容或 terminal 输出。
10. Ghostty surface 经多次前后台、旋转、键盘切换、attach/detach 后没有崩溃或明显
    累积的 GPU 内存。

## Review 建议顺序

1. 先裁决 Cloud 是否中继 Terminal data plane。
2. 再裁决身份/配对与 E2EE 边界;它们决定 Cloud 能看到什么。
3. 确认锁屏通知允许的动作,尤其是否禁止直接 Allow。
4. 确认 Phase 1 是否可以在 Cloud 前独立发布。
5. 运行 Ghostty spike,用实测结果确定 iOS 最低版本和 renderer 路线。
6. 最后把确认的需求拆成:iOS spec、Cloud spec、server connector spec、Cloud protocol
   spec 与里程碑计划。

## 参考资料

- [octoterm wire protocol](../protocol.md)
- [octoterm v1 设计](../superpowers/specs/2026-08-16-octoterm-design.md)
- [octoterm agent 集成设计](../superpowers/specs/2026-08-18-octoterm-agent-integration-design.md)
- [Ghostty internal embedder header](https://raw.githubusercontent.com/ghostty-org/ghostty/main/include/ghostty.h)
- [Ghostty iOS / iPhone discussion](https://github.com/ghostty-org/ghostty/discussions/9285)
- [Apple:Choosing the right networking API](https://developer.apple.com/documentation/technotes/tn3151-choosing-the-right-networking-api)
- [Apple:Setting up a remote notification server](https://developer.apple.com/documentation/usernotifications/setting-up-a-remote-notification-server)
- [Apple:Handling notifications and notification-related actions](https://developer.apple.com/documentation/usernotifications/handling-notifications-and-notification-related-actions)
