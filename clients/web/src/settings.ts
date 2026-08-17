/**
 * 设置面板:一个分 tab 的 modal。
 *
 * 所有改动**即时生效**(不设「保存」按钮)—— 主题和字体是所见即所得的东西,
 * 让用户改一格看一眼比改完一整屏再提交有用得多。持久化由 host.set 负责。
 *
 * 面板整个用 DOM API 搭,不用 innerHTML 拼用户数据:主题名来自导入的 JSON,
 * 拼字符串就是一个 XSS 口子。
 */
import type { ITheme } from "@xterm/xterm";
import {
  type OctoConfig,
  defaultConfig,
  exportConfigJson,
  importConfigJson,
  sanitizeTheme,
  DEFAULT_FONT_FAMILY,
} from "./config";
import { knownThemes, loadCatalog, catalogLoaded, resolveTheme } from "./theme-catalog";

export interface SettingsHost {
  get(): OctoConfig;
  /** 应用并持久化。 */
  set(cfg: OctoConfig): void;
}

type Tab = "theme" | "font" | "terminal" | "io";

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

/** 一行「标签 + 控件」。 */
function row(label: string, control: HTMLElement, hint?: string): HTMLElement {
  const r = el("label", "set-row");
  const l = el("span", "set-label", label);
  if (hint) {
    const h = el("span", "set-hint", hint);
    l.appendChild(h);
  }
  r.append(l, control);
  return r;
}

function numberInput(value: number, min: number, max: number, step: number): HTMLInputElement {
  const i = el("input");
  i.type = "number";
  i.min = String(min);
  i.max = String(max);
  i.step = String(step);
  i.value = String(value);
  return i;
}

function select<T extends string>(options: readonly T[], value: T): HTMLSelectElement {
  const s = el("select");
  for (const o of options) {
    const opt = el("option", undefined, o);
    opt.value = o;
    s.appendChild(opt);
  }
  s.value = value;
  return s;
}

function checkbox(value: boolean): HTMLInputElement {
  const i = el("input");
  i.type = "checkbox";
  i.checked = value;
  return i;
}

/** 主题色块条:8 个常用色 + 前景/背景,用来在不点开的情况下认出一个主题。 */
function swatch(theme: ITheme): HTMLElement {
  const s = el("span", "swatch");
  s.style.background = theme.background ?? "#000";
  for (const k of ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "foreground"] as const) {
    const d = el("i");
    d.style.background = theme[k] ?? "transparent";
    s.appendChild(d);
  }
  return s;
}

/** 从字体栈里取第一个族名,用来做「系统装没装」的探测。 */
function firstFamily(stack: string): string {
  return (stack.split(",")[0] ?? "").trim().replace(/^["']|["']$/g, "");
}

export function mountSettings(host: SettingsHost): { open: () => void } {
  const overlay = el("div", "set-overlay");
  overlay.hidden = true;
  const modal = el("div", "set-modal");
  const head = el("div", "set-head");
  const title = el("span", "set-title", "设置");
  const closeBtn = el("button", "set-close", "✕");
  closeBtn.title = "关闭 (Esc)";
  head.append(title, closeBtn);

  const tabsBar = el("div", "set-tabs");
  const body = el("div", "set-body");
  modal.append(head, tabsBar, body);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  let tab: Tab = "theme";
  const TABS: [Tab, string][] = [
    ["theme", "主题"],
    ["font", "字体"],
    ["terminal", "终端"],
    ["io", "导入 / 导出"],
  ];

  /** 改一格配置:浅合并进当前配置并立即应用。 */
  const patch = (fn: (c: OctoConfig) => OctoConfig) => host.set(fn(host.get()));

  function renderTabs() {
    tabsBar.innerHTML = "";
    for (const [id, label] of TABS) {
      const b = el("button", "set-tab" + (id === tab ? " active" : ""), label);
      b.addEventListener("click", () => {
        tab = id;
        render();
      });
      tabsBar.appendChild(b);
    }
  }

  /* ---------- 主题 ---------- */
  let themeFilter = "";

  function renderTheme(): HTMLElement {
    const wrap = el("div", "set-pane");
    const search = el("input", "set-search");
    search.type = "search";
    search.placeholder = catalogLoaded()
      ? `搜索 ${Object.keys(knownThemes()).length} 个主题…`
      : "搜索主题(正在加载完整目录…)";
    search.value = themeFilter;
    search.addEventListener("input", () => {
      themeFilter = search.value;
      grid.replaceChildren(...themeCards());
    });
    const grid = el("div", "theme-grid");

    function themeCards(): HTMLElement[] {
      const q = themeFilter.trim().toLowerCase();
      const current = host.get().theme.name;
      const entries = Object.entries(knownThemes())
        .filter(([name]) => q === "" || name.toLowerCase().includes(q))
        .sort(([a], [b]) => a.localeCompare(b));
      if (entries.length === 0) return [el("div", "set-empty", "没有匹配的主题")];
      return entries.slice(0, 400).map(([name, colors]) => {
        const card = el("button", "theme-card" + (name === current ? " active" : ""));
        card.append(swatch(colors), el("span", "theme-name", name));
        card.addEventListener("click", () => {
          patch((c) => ({ ...c, theme: { name, colors: sanitizeTheme(colors) } }));
          render();
        });
        return card;
      });
    }

    grid.replaceChildren(...themeCards());
    wrap.append(search, grid);

    if (!catalogLoaded()) {
      void loadCatalog().then(() => {
        if (tab === "theme") render();
      });
    }

    const note = el(
      "div",
      "set-note",
      "主题数据来自 mbadolato/iTerm2-Color-Schemes(MIT)。想要目录里没有的配色," +
        "在「导入 / 导出」里直接改 theme.colors 即可。",
    );
    wrap.appendChild(note);
    return wrap;
  }

  /* ---------- 字体 ---------- */
  function renderFont(): HTMLElement {
    const cfg = host.get();
    const wrap = el("div", "set-pane");

    const family = el("input", "set-wide");
    family.type = "text";
    family.value = cfg.font.family;
    family.spellcheck = false;
    const avail = el("div", "set-note");
    const checkAvail = (stack: string) => {
      const first = firstFamily(stack);
      const generic = ["monospace", "ui-monospace", "serif", "sans-serif", "system-ui"].includes(first);
      if (first === "") {
        avail.textContent = "";
      } else if (generic || document.fonts?.check?.(`12px "${first}"`)) {
        avail.textContent = `✓ 首选族「${first}」可用`;
        avail.className = "set-note ok";
      } else {
        avail.textContent = `⚠ 系统里找不到「${first}」,会回落到栈里的下一个`;
        avail.className = "set-note warn";
      }
    };
    checkAvail(cfg.font.family);
    family.addEventListener("change", () => {
      patch((c) => ({ ...c, font: { ...c.font, family: family.value } }));
      // sanitize 可能改写了输入(剔非法字符/空值回退),回读一次让输入框说实话
      family.value = host.get().font.family;
      checkAvail(family.value);
    });

    const reset = el("button", undefined, "恢复默认字体栈");
    reset.addEventListener("click", () => {
      patch((c) => ({ ...c, font: { ...c.font, family: DEFAULT_FONT_FAMILY } }));
      render();
    });

    const size = numberInput(cfg.font.size, 6, 48, 1);
    size.addEventListener("input", () =>
      patch((c) => ({ ...c, font: { ...c.font, size: Number(size.value) } })),
    );
    const lh = numberInput(cfg.font.lineHeight, 0.8, 3, 0.05);
    lh.addEventListener("input", () =>
      patch((c) => ({ ...c, font: { ...c.font, lineHeight: Number(lh.value) } })),
    );
    const ls = numberInput(cfg.font.letterSpacing, -5, 10, 0.5);
    ls.addEventListener("input", () =>
      patch((c) => ({ ...c, font: { ...c.font, letterSpacing: Number(ls.value) } })),
    );
    const weights = ["normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900"] as const;
    const w = select(weights, String(cfg.font.weight) as (typeof weights)[number]);
    w.addEventListener("change", () =>
      patch((c) => ({ ...c, font: { ...c.font, weight: w.value as never } })),
    );
    const wb = select(weights, String(cfg.font.weightBold) as (typeof weights)[number]);
    wb.addEventListener("change", () =>
      patch((c) => ({ ...c, font: { ...c.font, weightBold: wb.value as never } })),
    );

    const preview = el("div", "font-preview");
    preview.style.fontFamily = cfg.font.family;
    preview.style.fontSize = `${cfg.font.size}px`;
    preview.style.lineHeight = String(cfg.font.lineHeight);
    preview.style.letterSpacing = `${cfg.font.letterSpacing}px`;
    preview.style.background = cfg.theme.colors.background ?? "#000";
    preview.style.color = cfg.theme.colors.foreground ?? "#fff";
    preview.textContent = "iIlL1 oO0 `'\" ─┼┤├ 中文对齐 => != >= 0x1F ~$#@";

    wrap.append(
      row("字体栈", family, "CSS font-family,逗号分隔"),
      avail,
      row("", reset),
      row("字号", size, "px"),
      row("行高", lh, "倍"),
      row("字距", ls, "px"),
      row("常规字重", w),
      row("加粗字重", wb),
      preview,
    );
    return wrap;
  }

  /* ---------- 终端行为 ---------- */
  function renderTerminal(): HTMLElement {
    const cfg = host.get();
    const wrap = el("div", "set-pane");

    const cs = select(["block", "underline", "bar"] as const, cfg.terminal.cursorStyle);
    cs.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, cursorStyle: cs.value as never } })),
    );
    const cis = select(
      ["outline", "block", "bar", "underline", "none"] as const,
      cfg.terminal.cursorInactiveStyle,
    );
    cis.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, cursorInactiveStyle: cis.value as never } })),
    );
    const blink = checkbox(cfg.terminal.cursorBlink);
    blink.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, cursorBlink: blink.checked } })),
    );
    const sb = numberInput(cfg.terminal.scrollback, 0, 200_000, 500);
    sb.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, scrollback: Number(sb.value) } })),
    );
    const glyphs = checkbox(cfg.terminal.customGlyphs);
    glyphs.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, customGlyphs: glyphs.checked } })),
    );
    const contrast = numberInput(cfg.terminal.minimumContrastRatio, 1, 21, 0.5);
    contrast.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, minimumContrastRatio: Number(contrast.value) } })),
    );
    const boldBright = checkbox(cfg.terminal.drawBoldTextInBrightColors);
    boldBright.addEventListener("change", () =>
      patch((c) => ({
        ...c,
        terminal: { ...c.terminal, drawBoldTextInBrightColors: boldBright.checked },
      })),
    );

    const follow = checkbox(cfg.ui.followThemeColors);
    follow.addEventListener("change", () => {
      patch((c) => ({ ...c, ui: { ...c.ui, followThemeColors: follow.checked } }));
    });
    const prev = checkbox(cfg.ui.sidebarPreview);
    prev.addEventListener("change", () =>
      patch((c) => ({ ...c, ui: { ...c.ui, sidebarPreview: prev.checked } })),
    );
    const webgl = checkbox(cfg.ui.webgl);
    webgl.addEventListener("change", () =>
      patch((c) => ({ ...c, ui: { ...c.ui, webgl: webgl.checked } })),
    );

    wrap.append(
      row("光标形状", cs),
      row("失焦光标", cis),
      row("光标闪烁", blink),
      row("回滚行数", sb, "行"),
      row("内置字形绘制", glyphs, "自己画 box-drawing / powerline,不依赖 Nerd Font"),
      row("最小对比度", contrast, "1 = 不干预;提高可读性但会改写主题色"),
      row("加粗用亮色", boldBright),
      el("div", "set-sep", "界面"),
      row("界面跟随主题配色", follow),
      row("侧边栏会话预览", prev),
      row("WebGL 渲染器", webgl, "关闭则回落到 DOM 渲染器"),
    );
    return wrap;
  }

  /* ---------- 导入 / 导出 ---------- */
  function renderIo(): HTMLElement {
    const wrap = el("div", "set-pane");
    const ta = el("textarea", "set-json");
    ta.spellcheck = false;
    ta.value = exportConfigJson(host.get());
    const status = el("div", "set-note");

    const say = (msg: string, kind: "ok" | "warn" | "" = "") => {
      status.textContent = msg;
      status.className = "set-note" + (kind ? ` ${kind}` : "");
    };

    const apply = (text: string, source: string) => {
      let result;
      try {
        result = importConfigJson(text, resolveTheme);
      } catch (err) {
        say(`${source}失败:不是合法的 JSON(${(err as Error).message})`, "warn");
        return;
      }
      host.set(result.config);
      ta.value = exportConfigJson(result.config);
      if (result.warnings.length === 0) {
        say(`${source}成功`, "ok");
      } else {
        say(`${source}成功,但有 ${result.warnings.length} 处被修正:\n· ` + result.warnings.join("\n· "), "warn");
      }
    };

    const applyBtn = el("button", undefined, "应用上面的 JSON");
    applyBtn.addEventListener("click", () => apply(ta.value, "导入"));

    const download = el("button", undefined, "导出为文件");
    download.addEventListener("click", () => {
      const blob = new Blob([exportConfigJson(host.get())], { type: "application/json" });
      const a = el("a");
      a.href = URL.createObjectURL(blob);
      a.download = "octoterm-config.json";
      a.click();
      // 立刻 revoke 会让部分浏览器来不及发起下载,挂到下一帧之后
      setTimeout(() => URL.revokeObjectURL(a.href), 10_000);
    });

    const file = el("input");
    file.type = "file";
    file.accept = "application/json,.json";
    file.hidden = true;
    file.addEventListener("change", async () => {
      const f = file.files?.[0];
      if (!f) return;
      apply(await f.text(), "从文件导入");
      file.value = "";
    });
    const upload = el("button", undefined, "从文件导入");
    upload.addEventListener("click", () => file.click());

    const reset = el("button", "danger", "恢复全部默认");
    reset.addEventListener("click", () => {
      if (!confirm("把主题、字体、终端设置全部恢复为默认?")) return;
      host.set(defaultConfig());
      ta.value = exportConfigJson(host.get());
      say("已恢复默认", "ok");
    });

    const bar = el("div", "set-bar");
    bar.append(applyBtn, download, upload, reset, file);
    wrap.append(
      el(
        "div",
        "set-note",
        "配置就是下面这段 JSON,可以直接改。theme.colors 是 xterm.js 的 ITheme," +
          "所以任何 iTerm2 / Windows Terminal 配色都能手工贴进来。",
      ),
      ta,
      bar,
      status,
    );
    return wrap;
  }

  function render() {
    renderTabs();
    const pane =
      tab === "theme" ? renderTheme()
      : tab === "font" ? renderFont()
      : tab === "terminal" ? renderTerminal()
      : renderIo();
    body.replaceChildren(pane);
  }

  const close = () => {
    overlay.hidden = true;
  };
  closeBtn.addEventListener("click", close);
  overlay.addEventListener("click", (ev) => {
    if (ev.target === overlay) close();
  });
  document.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape" && !overlay.hidden) {
      ev.stopPropagation();
      close();
    }
  });

  return {
    open() {
      overlay.hidden = false;
      render();
    },
  };
}
