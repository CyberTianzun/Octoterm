# octoterm 手动验收清单

前置:`cd clients/web && npm run build`,`cargo run -p octoterm-server`,记下打印的 token,
浏览器打开 `http://127.0.0.1:7683/#token=<token>`。

## 桌面浏览器
- [ ] 列表页显示会话卡片,预览可见
- [ ] 点 + → 弹出启动项菜单;选一条 → 卡片实时出现,名字就是启动项的名字;
      Rename/Kill 生效且另一浏览器标签页同步更新

## 新建会话菜单(点侧边栏的 +)
菜单内容来自服务端扫描(内置默认 shell + config.toml 的 `[[launcher]]` +
iTerm2 / Windows Terminal 的 profile),见 protocol.md §2.2。
- [ ] **取消不建会话**:分别用 Esc、点菜单外面、再点一次 + → 菜单关闭,
      会话列表**不增加**(这是原来 `prompt()` 点「否」照样建会话的那个 bug)
- [ ] 第一条永远是内置默认 shell,分组标题按来源分(内置 / iTerm2 / …)
- [ ] 键盘可用:↑↓ 移动、回车建会话、Esc 取消;筛选框输入能过滤
- [ ] 选一条带工作目录的 profile → 新会话的 `pwd` 就是那个目录
- [ ] 选一条工作目录**在本机不存在**的 profile(比如 iTerm2 里写了 Windows 路径)
      → 仍能建会话,落在 `$HOME`,服务端日志有一条 WARN
- [ ] 在 iTerm2 里新加一个带自定义命令的 profile,**不重启 octoterm** → 重开菜单就能看到
- [ ] 在 config.toml 里加 `[[launcher]]`(重启服务端)→ 出现在「自定义」分组
- [ ] 停掉服务端再点 + → 菜单里仍有一条「默认 shell」兜底,不是空菜单
- [ ] `curl http://127.0.0.1:7683/api/launchers` 不带 token → 401
- [ ] Attach → shell 可交互;`ls --color`、`vim`、`htop` 渲染正常
- [ ] 拉伸窗口 → 终端随 fit 重排,`tput cols` 值同步变化
- [ ] `cat` 一个数 MB 文件 → 页面不卡死,结束后画面正确(resync 生效)
- [ ] 杀掉网络(关 wifi 数秒再恢复)→ 顶部 reconnecting 条出现后消失,会话内容延续
- [ ] 关闭标签页重开 → 会话仍在,attach 恢复现场

## 多端同时 attach 一个会话(默认 window-size = smallest)
- [ ] 大窗口 attach 后,小窗口(另开一个窄浏览器窗口/手机)再 attach → 两边同时缩到
      小的那个尺寸,`tput cols` 一致;大窗口多出来的地方留白居中,画面不错行
- [ ] 拖动其中任一窗口 → 两边同步跟随,没有互相抢尺寸的抖动(vim/htop 不反复重绘)
- [ ] 小窗口那端 detach 或关掉标签页 → 大窗口立即恢复到自己的尺寸
      (直接断网这种半开连接要等心跳超时,最多 90s,见 protocol.md G8)
- [ ] `--window-size largest` / `latest` 启动 → 行为分别变为跟随最大 / 跟随最近操作的一端

## 首次打开:默认主题跟随系统亮暗
清 localStorage(或用无痕窗口)才算「首次」—— 存过配置之后就以存的为准,不再看系统。
- [ ] 系统设为**深色** → 清 localStorage 后打开 → 主题是 2026 Dark
- [ ] 系统设为**浅色** → 清 localStorage 后打开 → 主题是 2026 Light,整个界面是亮的
- [ ] 首次打开后**什么都不改**,切换系统亮暗再刷新 → 主题跟着换
      (还没有用户选择可言,跟随系统是对的)
- [ ] 手动选一个主题(或只改字号)之后,再切换系统亮暗并刷新 → 主题**不变**
      (已经有用户选择了,系统偏好不该覆盖它)
- [ ] 「恢复全部默认」→ 按**当下**的系统亮暗回到 2026 Dark 或 2026 Light,
      提示语里会写明落到了哪个

## 外观配置(侧边栏 ⚙)
- [ ] 「主题」tab 首屏立刻能选内置主题;搜索框提示从「正在加载完整目录」
      变成「搜索 603 个主题」→ 全量目录拉到了
- [ ] 断网后打开「主题」tab → 只剩内置主题,不报错、当前主题不受影响
- [ ] 换一个亮色主题(如 2026 Light)→ 终端**和侧边栏、按钮、
      滚动条**一起变亮;文字不会变成白底白字
- [ ] 改字号 → 终端立刻重排,`tput cols` 跟着变(字号变了能放下的列数就变了)
- [ ] 字体栈填一个系统没有的族名 → 提示「找不到」,但仍回落渲染,不白屏
- [ ] 刷新页面 → 主题/字体全部保持;换一个浏览器则是默认配置
- [ ] 关掉「侧边栏会话预览」→ 预览消失,新建会话时也不再出现
- [ ] 关掉「WebGL 渲染器」→ 画面不变(回落 DOM 渲染器),`cat` 大文件仍不卡死
- [ ] **渲染器释放路径**(WebGL 开着时):在会话间来回切换若干次、再关掉终端回到空态
      → 控制台无异常,切回去内容正常。addon 与 @xterm/xterm 是同版本发布的强耦合
      且未声明 peerDependencies,版本错配的症状正是在这里抛
      `Cannot read properties of undefined`(见 main.ts 顶部注释)

## 配置导入 / 导出
- [ ] 「导出为文件」下载 octoterm-config.json,内容与面板里那段 JSON 一致
- [ ] 改主题后重新导出 → theme.colors 跟着变(导出自带全部颜色,不只是主题名)
- [ ] 把导出的文件在另一个浏览器「从文件导入」→ 外观完全一致
- [ ] 手工把 JSON 改成 `{"theme":{"name":"Dracula"}}`(只给名字不给颜色)→ 应用后
      正确解析成 Dracula
- [ ] 故意填坏值(`"font":{"size":9999}`、颜色写成 `"red"`)→ 提示「有 N 处被修正」
      并收敛到合法值,界面不崩
- [ ] 贴一段非 JSON → 提示不是合法 JSON,当前配置不受影响
- [ ] 「恢复全部默认」→ 字体栈回到默认;主题见上面「首次打开」那节

## 界面语言(设置 → 界面 → 语言)
词条见 `clients/web/src/i18n.ts`;键位对齐、占位符对齐、index.html 引用的 key 都有
自动化测试(`test/i18n.test.mjs`),这里只验人眼才看得出来的部分。
- [ ] 清 localStorage,把浏览器首选语言设成中文 → 打开就是中文界面;
      设成英文(或任何不支持的语言,如法语)→ 打开就是英文界面
- [ ] 语言选「English」→ **整页**立刻变英文:侧边栏页脚、⚙/+/☰ 的 tooltip、
      空态提示、设置面板所有 tab 与其中的标签、新建会话菜单的筛选框和分组标题
- [ ] 「界面」tab 里语言是第一行;窗口窄到手机宽度时 5 个 tab 换行而不是溢出
- [ ] 切语言后会话卡片上的时间跟着换格式(中文 `2026/8/18` vs 英文 `8/18/2026`)
- [ ] 切成英文后刷新 → 仍是英文(存进了配置);再选「跟随浏览器」→ 回到系统语言
- [ ] 英文界面下故意导入一段坏 JSON → 「N 处被修正」的**每条警告**都是英文,
      没有中英混排
- [ ] 英文界面下停掉服务端 → 「新建会话」菜单里的兜底项显示 "Default shell",
      分组标题是 "Built-in";iTerm2 / Windows Terminal 这类品牌名两种语言下都不翻译
- [ ] 断网让连接失败 → 顶部横幅和侧边栏页脚的连接状态都是当前语言
- [ ] `<html lang>` 跟着当前语言变(开发者工具里看)—— 影响浏览器的断词和朗读

## 移动浏览器(iOS Safari / Android Chrome)
- [ ] 列表页可读可点
- [ ] Attach 后软键盘弹出,终端随 visualViewport 缩放,输入回显正常
- [ ] 触摸滚动查看回滚历史
- [ ] 锁屏 30 秒回来 → 自动重连恢复
- [ ] 设置面板在窄屏下可用,主题网格能滚动

## Agent 集成:Codex 端到端(需要人工点头,自动化测不了)

Codex 逐条用 `trusted_hash` 给 hook 上闸,**必须由你在它自己的 TUI 里 `/hooks` review
过才生效**。我们不伪造那个 hash —— 那道闸防的正是「第三方悄悄让 Codex 执行任意命令」,
而 octoterm 恰好就是那个第三方。所以这条链路只能手动验。

自动化已经覆盖的部分(`cargo test -p octoterm-server --test agent_codex`):装出来的
hooks.json 形状、幂等与还原、所有权判定、`hook` 子命令对真实 server 的端到端、以及
三条降级路径。**下面要验的是自动化够不到的那一段**:Codex 真的会调用它,以及决策
真的会被它采纳。

### 0. 隔离(重要)

全程不碰你真实的 `~/.codex`。两个变量分工不同,别弄混:

- **server 的 `HOME`** 决定安装器往哪写(它拼的是 `<HOME>/.codex/hooks.json`);
- **codex 的 `CODEX_HOME`** 决定 Codex 自己读哪里。

> 已知缺口:安装器**不读 `$CODEX_HOME`**,只认 `<HOME>/.codex`
> (Claude Code 那边不读 `$CLAUDE_CONFIG_DIR`,同一类问题)。下面靠「让两者指到
> 同一个地方」绕过去;要修的话是给 adapter 传一个可覆盖的配置目录。

```sh
export LAB=/tmp/octoterm-codex-lab
rm -rf "$LAB" && mkdir -p "$LAB/.codex"
cp ~/.codex/auth.json "$LAB/.codex/"          # 借用登录态,不改原文件
printf '[agents]\ninstall_enabled = true\n' > "$LAB/octoterm.toml"
```

### 1. 起 server

```sh
HOME="$LAB" cargo run -p octoterm-server -- \
  --port 7699 --token labtok --config "$LAB/octoterm.toml"
```

- [ ] 启动日志打印监听地址,没有报错

### 2. 预演,然后安装

```sh
curl -s -H "Authorization: Bearer labtok" \
  http://127.0.0.1:7699/api/agents/codex/plan | python3 -m json.tool
curl -s -X POST -H "Authorization: Bearer labtok" \
  http://127.0.0.1:7699/api/agents/codex/install
```

- [ ] `plan` 里 `install` 列出 6 个事件,路径是 `$LAB/.codex/hooks.json`
- [ ] `install` 返回 `changed: true`
- [ ] `$LAB/.codex/hooks.json` 里每条是 `type: "command"`,命令串形如
      `"…/octoterm-server" hook http://127.0.0.1:7699/hook/codex/<事件>`
- [ ] group **不带 `matcher`**(Codex 接受的就是这个形状)
- [ ] `GET /api/agents` 里 codex 那条的 `activation` 是 `"codex-hooks-review"`

### 3. 在**托管会话里**跑 Codex

这一步是关键:必须从 octoterm 的会话里启动,`OCTOTERM_SESSION_ID` /
`OCTOTERM_HOOK_TOKEN` 是 spawn 时注入的 —— 在别处起的 Codex 拿不到,hook 会自己
静默退出(这正是「只管托管会话」那条边界)。

浏览器打开 `http://127.0.0.1:7699/#token=labtok`,新建一个会话,在里面:

```sh
echo "$OCTOTERM_SESSION_ID"        # 应当是个数字,不是空
export CODEX_HOME=$LAB/.codex
cd /tmp && codex
```

- [ ] `echo $OCTOTERM_SESSION_ID` 有值(没值就说明不是从 octoterm 会话里起的)

### 4. 过 Codex 自己的闸

在 Codex 的 TUI 里:

```
/hooks
```

- [ ] 列出 6 条待审的 hook,命令串指向 octoterm 的二进制
- [ ] 逐条确认后,`$LAB/.codex/config.toml` 里出现
      `[hooks.state."…/hooks.json:<事件小写下划线>:0:0"]` 与 `trusted_hash`
- [ ] **审核之前**先随便让它跑个工具 → server 的 `/api/agents/sessions` **不该**出现
      codex 会话(闸没过,hook 根本不会被调用)

### 5. 遥测:状态跟着动

审核之后,在 Codex 里发一条普通指令(比如让它读个文件)。

```sh
curl -s -H "Authorization: Bearer labtok" http://127.0.0.1:7699/api/agents/sessions | python3 -m json.tool
```

- [ ] 出现 `agent_id: "codex"` 的会话,`session` 等于第 3 步那个数字
- [ ] 状态随操作变化:提交指令 → `thinking`,跑工具 → `working`
- [ ] octoterm 网页的会话列表上,那一行出现状态点

### 6. 决策:远程替它拍板

在 Codex 里让它做一件**需要授权**的事(例如写文件或跑一条不在允许列表里的命令)。

- [ ] Codex 卡住等待(它在等我们写响应)
- [ ] octoterm 网页顶部出现「有 AI 在等你回答」横幅,写着那个会话名
- [ ] `GET /api/agents/pending` 里能看到 `tool_name` 与 `tool_input`
- [ ] **换一个设备**(手机浏览器开同一个地址)也能看到同一条 —— 这就是这个功能的意义
- [ ] 点「允许」→ Codex 立刻继续执行
- [ ] 再来一次,点「拒绝」→ Codex 报告被拒绝,没有执行

降级也顺手验一下:

- [ ] 再触发一次授权,然后**把 server Ctrl-C 掉** → Codex 不该卡死,应当回落到它自己的
      审批提示(hook 连不上 = 无决定)
- [ ] 触发一次授权后**谁也不答**,等到超时 → 同上,回落到 Codex 自己的提示

### 7. 卸载与清场

```sh
curl -s -X POST -H "Authorization: Bearer labtok" \
  http://127.0.0.1:7699/api/agents/codex/uninstall
```

- [ ] `$LAB/.codex/hooks.json` 里我们的条目消失;别人的条目(如果你造过)原样还在
- [ ] 原本没有 hooks.json 的话,卸载后会留下一个空的 `{}` —— 那是安装时创建的空壳,
      内容上「我们没来过」,但文件本身还在(已知的小残留,不影响 Codex)
- [ ] 全程结束后 `stat -f %Sm ~/.codex/config.toml` 与 `~/.codex/hooks.json` 的时间**没变**
- [ ] `ls ~/Library/Application\ Support/octoterm/agent-backups` 不存在或没有新增
      (备份都落在 `$LAB` 下)

### 排查:一步都没触发怎么办

按这个顺序看,这几条都是实际踩过的:

1. **`/hooks` 审核过了吗** —— 没过的话 hook 完全不会被调用,而且**毫无提示**;
2. **是从托管会话里起的 Codex 吗** —— `echo $OCTOTERM_SESSION_ID` 为空就说明不是,
   hook 会静默退出、连包都不发;
3. **`CODEX_HOME` 指对了吗** —— 指错了 Codex 读的是另一份 hooks.json;
4. **端口对得上吗** —— 换过端口的话已写入的命令串还指着旧端口,
   `GET /api/agents` 会把它报成 `stale-port`,重装一次即可;
5. **新目录第一次跑会不会卡在信任对话框** —— Codex 和 Claude Code 都有工作区信任
   确认,交互式第一次跑会停在那里(这条在 Claude Code 的端到端里真卡过一次)。
