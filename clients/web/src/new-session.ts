/**
 * 「新建会话」菜单:一个锚在 + 按钮下方的弹出菜单。
 *
 * 它替掉的是 `prompt()`。原生弹窗有两个治不好的毛病:一是取消和输入空串在
 * `prompt() || null` 之下没法区分(点「否」照样建会话);二是浏览器对连续弹窗
 * 会静默抑制,表现成点了没反应。菜单这两个问题都不存在 —— **关闭菜单就是取消**,
 * 只有点中某一项才会建会话。
 *
 * 面板整个用 DOM API 搭,不拼 innerHTML:条目名字来自 iTerm2 / Windows Terminal
 * 的配置文件,拼字符串就是一个 XSS 口子(和 settings.ts 同样的理由)。
 */
import { type Launcher, providerLabel } from "./launchers";
import { subscribe, t } from "./i18n";

export interface NewSessionHost {
  /** 拉取启动项。每次打开都会调 —— 用户可能刚在 iTerm2 里加了个 profile。 */
  load(): Promise<Launcher[]>;
  pick(launcher: Launcher): void;
}

const el = <K extends keyof HTMLElementTagNameMap>(
  tag: K,
  cls?: string,
  text?: string,
): HTMLElementTagNameMap[K] => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
};

export function mountNewSessionMenu(
  anchor: HTMLElement,
  host: NewSessionHost,
): { open: () => void; close: () => void } {
  const menu = el("div", "ns-menu");
  menu.hidden = true;
  menu.setAttribute("role", "menu");

  const filter = el("input", "ns-filter");
  filter.type = "search";

  const list = el("div", "ns-list");
  menu.append(filter, list);
  document.body.appendChild(menu);

  let open = false;
  /** 上一次拉到的列表:再次打开时先拿它铺满,后台再刷新,避免每次都闪一下"载入中" */
  let cached: Launcher[] | null = null;
  /** 打开次数。异步结果回来时对不上就说明菜单已经关了/重开了,丢弃 */
  let generation = 0;
  let shown: Launcher[] = [];
  let selected = 0;

  function matches(l: Launcher, q: string): boolean {
    if (!q) return true;
    const hay = `${l.name}\n${l.detail}\n${providerLabel(l.provider)}`.toLowerCase();
    return hay.includes(q);
  }

  function render(all: Launcher[] | null) {
    list.replaceChildren();
    if (all === null) {
      list.appendChild(el("div", "ns-note", t("ns.loading")));
      shown = [];
      return;
    }
    const q = filter.value.trim().toLowerCase();
    shown = all.filter((l) => matches(l, q));
    if (shown.length === 0) {
      list.appendChild(el("div", "ns-note", t("ns.noMatch")));
      return;
    }
    if (selected >= shown.length) selected = shown.length - 1;
    if (selected < 0) selected = 0;

    let group: string | null = null;
    shown.forEach((l, i) => {
      if (l.provider !== group) {
        group = l.provider;
        list.appendChild(el("div", "ns-group", providerLabel(l.provider)));
      }
      const item = el("button", "ns-item" + (i === selected ? " sel" : ""));
      item.type = "button";
      item.setAttribute("role", "menuitem");
      item.append(el("span", "ns-name", l.name));
      // 命令预览带上工作目录:两条只差目录的 profile 光看名字分不出来
      const detail = l.cwd ? `${l.detail}  ·  ${l.cwd}` : l.detail;
      if (detail) item.append(el("span", "ns-detail", detail));
      item.addEventListener("mousemove", () => setSelected(i));
      item.addEventListener("click", () => choose(l));
      list.appendChild(item);
    });
    scrollSelectedIntoView();
  }

  function setSelected(i: number) {
    if (i === selected) return;
    selected = i;
    const items = list.querySelectorAll(".ns-item");
    items.forEach((n, idx) => n.classList.toggle("sel", idx === selected));
    scrollSelectedIntoView();
  }

  function scrollSelectedIntoView() {
    const node = list.querySelectorAll(".ns-item")[selected];
    node?.scrollIntoView({ block: "nearest" });
  }

  function move(delta: number) {
    if (shown.length === 0) return;
    // 绕回去:列表短的时候,从头往上一格直接跳到末尾比"卡住不动"顺手
    setSelected((selected + delta + shown.length) % shown.length);
  }

  function choose(l: Launcher) {
    close();
    host.pick(l);
  }

  /** 贴着按钮右下角展开,并且不许溢出视口(移动端 sidebar 是抽屉,宽度很紧张)。 */
  function place() {
    const rect = anchor.getBoundingClientRect();
    const width = Math.min(340, window.innerWidth - 16);
    menu.style.width = `${width}px`;
    menu.style.left = `${Math.max(8, Math.min(rect.right - width, window.innerWidth - width - 8))}px`;
    menu.style.top = `${rect.bottom + 6}px`;
    menu.style.maxHeight = `${Math.max(160, window.innerHeight - rect.bottom - 24)}px`;
  }

  function openMenu() {
    if (open) {
      close();
      return; // 再点一次 + 是收起,不是重开
    }
    open = true;
    const gen = ++generation;
    filter.value = "";
    selected = 0;
    menu.hidden = false;
    place();
    render(cached);
    filter.focus();
    // 无论有没有缓存都刷一次:缓存只是为了不闪,不是真相
    host.load().then((all) => {
      if (!open || gen !== generation) return;
      cached = all;
      render(all);
    });
  }

  function close() {
    if (!open) return;
    open = false;
    generation++;
    menu.hidden = true;
    // 焦点还回按钮:用键盘操作的人不会因为关个菜单就丢了位置
    anchor.focus();
  }

  filter.addEventListener("input", () => {
    selected = 0;
    render(cached);
  });

  menu.addEventListener("keydown", (ev) => {
    switch (ev.key) {
      case "Escape":
        ev.preventDefault();
        close();
        break;
      case "ArrowDown":
        ev.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        ev.preventDefault();
        move(-1);
        break;
      case "Enter": {
        ev.preventDefault();
        const l = shown[selected];
        if (l) choose(l);
        break;
      }
    }
  });

  // 点菜单和按钮以外的任何地方 = 取消。用 pointerdown 而不是 click:
  // click 要等按键抬起,中间那段时间菜单还开着,看起来像没反应。
  document.addEventListener("pointerdown", (ev) => {
    if (!open) return;
    const target = ev.target as Node;
    if (!menu.contains(target) && !anchor.contains(target)) close();
  });
  // 这里**故意不监听 focusout**:点条目时 pointerdown 会先把焦点从筛选框挪走,
  // 若借 focusout 关菜单,条目会在 click 派发之前被 hidden 掉,点了没反应 ——
  // 正是要修的那个毛病。取消靠上面的外部 pointerdown 和 Escape 就够了。
  // 视口一变,之前算好的位置就不对了;重新定位比让菜单飘到别处强
  window.addEventListener("resize", () => open && place());

  /** 语言相关的静态文案。挂载时先跑一次,之后每次切语言再跑。 */
  function applyStaticText() {
    filter.placeholder = t("ns.filter");
    filter.setAttribute("aria-label", t("ns.filterAria"));
  }
  applyStaticText();
  // 切语言时连分组名(providerLabel)一起变,所以展开着的列表要重绘。
  // 缓存里的兜底项文案也过期了,一并丢掉,下次打开重新拉。
  subscribe(() => {
    applyStaticText();
    if (cached?.some((l) => l.id === "fallback:default")) cached = null;
    if (open) render(cached);
  });

  anchor.addEventListener("click", openMenu);

  return { open: openMenu, close };
}
