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
  systemDefaultConfig,
  exportConfigJson,
  importConfigJson,
  sanitizeTheme,
  DEFAULT_FONT_FAMILY,
} from "./config";
import { knownThemes, loadCatalog, catalogLoaded, resolveTheme } from "./theme-catalog";
import { LOCALES, LOCALE_NAMES, type LocalePref, type MsgKey, subscribe, t } from "./i18n";

export interface SettingsHost {
  get(): OctoConfig;
  /** 应用并持久化。 */
  set(cfg: OctoConfig): void;
}

type Tab = "theme" | "font" | "terminal" | "ui" | "io";

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

/** `label` 缺省时直接显示取值本身(字重那种 CSS 关键字不需要翻译)。 */
function select<T extends string>(
  options: readonly T[],
  value: T,
  label?: (o: T) => string,
): HTMLSelectElement {
  const s = el("select");
  for (const o of options) {
    const opt = el("option", undefined, label ? label(o) : o);
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

/** 光标形状枚举的显示名。block / outline 这些是 xterm 的内部取值,直接摆给
 *  用户看等于没说。字重那种本身就是 CSS 关键字的枚举则保持原样。 */
const CURSOR_LABELS: Record<string, MsgKey> = {
  block: "cursor.block",
  underline: "cursor.underline",
  bar: "cursor.bar",
  outline: "cursor.outline",
  none: "cursor.none",
};

const cursorLabel = (v: string): string => {
  const key = CURSOR_LABELS[v];
  return key ? t(key) : v;
};

/** 从字体栈里取第一个族名,用来做「系统装没装」的探测。 */
function firstFamily(stack: string): string {
  return (stack.split(",")[0] ?? "").trim().replace(/^["']|["']$/g, "");
}

export function mountSettings(host: SettingsHost): { open: () => void } {
  const overlay = el("div", "set-overlay");
  overlay.hidden = true;
  const modal = el("div", "set-modal");
  const head = el("div", "set-head");
  const title = el("span", "set-title");
  const closeBtn = el("button", "set-close", "✕");
  head.append(title, closeBtn);

  const tabsBar = el("div", "set-tabs");
  const body = el("div", "set-body");
  modal.append(head, tabsBar, body);
  overlay.appendChild(modal);
  document.body.appendChild(overlay);

  let tab: Tab = "theme";
  // 标签文案在 renderTabs 里现取:切语言后重绘就跟着变了
  const TABS: [Tab, MsgKey][] = [
    ["theme", "settings.tab.theme"],
    ["font", "settings.tab.font"],
    ["terminal", "settings.tab.terminal"],
    ["ui", "settings.tab.ui"],
    ["io", "settings.tab.io"],
  ];

  /** 改一格配置:浅合并进当前配置并立即应用。 */
  const patch = (fn: (c: OctoConfig) => OctoConfig) => host.set(fn(host.get()));

  function renderTabs() {
    tabsBar.innerHTML = "";
    for (const [id, key] of TABS) {
      const b = el("button", "set-tab" + (id === tab ? " active" : ""), t(key));
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
      ? t("settings.theme.search", { n: Object.keys(knownThemes()).length })
      : t("settings.theme.searchLoading");
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
      if (entries.length === 0) return [el("div", "set-empty", t("settings.theme.noMatch"))];
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

    const note = el("div", "set-note", t("settings.theme.note"));
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
        avail.textContent = t("settings.font.available", { name: first });
        avail.className = "set-note ok";
      } else {
        avail.textContent = t("settings.font.missing", { name: first });
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

    const reset = el("button", undefined, t("settings.font.reset"));
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
      row(t("settings.font.family"), family, t("settings.font.familyHint")),
      avail,
      row("", reset),
      row(t("settings.font.size"), size, t("unit.px")),
      row(t("settings.font.lineHeight"), lh, t("unit.times")),
      row(t("settings.font.letterSpacing"), ls, t("unit.px")),
      row(t("settings.font.weight"), w),
      row(t("settings.font.weightBold"), wb),
      preview,
    );
    return wrap;
  }

  /* ---------- 终端行为 ---------- */
  function renderTerminal(): HTMLElement {
    const cfg = host.get();
    const wrap = el("div", "set-pane");

    const cs = select(["block", "underline", "bar"] as const, cfg.terminal.cursorStyle, cursorLabel);
    cs.addEventListener("change", () =>
      patch((c) => ({ ...c, terminal: { ...c.terminal, cursorStyle: cs.value as never } })),
    );
    const cis = select(
      ["outline", "block", "bar", "underline", "none"] as const,
      cfg.terminal.cursorInactiveStyle,
      cursorLabel,
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

    wrap.append(
      row(t("settings.term.cursorStyle"), cs),
      row(t("settings.term.cursorInactive"), cis),
      row(t("settings.term.cursorBlink"), blink),
      row(t("settings.term.scrollback"), sb, t("unit.lines")),
      row(t("settings.term.customGlyphs"), glyphs, t("settings.term.customGlyphsHint")),
      row(t("settings.term.contrast"), contrast, t("settings.term.contrastHint")),
      row(t("settings.term.boldBright"), boldBright),
    );
    return wrap;
  }

  /* ---------- 界面 ---------- */

  /**
   * 语言排在第一行,而且整个 tab 就叫「界面」。
   *
   * 之前这几项挂在「终端」tab 的一个分隔线下面 —— 找不到。这三个开关本来也
   * 不是终端行为(它们管的是侧边栏、渲染器、配色外溢),搬出来两件事一起修。
   */
  function renderUi(): HTMLElement {
    const cfg = host.get();
    const wrap = el("div", "set-pane");

    const langs: readonly LocalePref[] = ["auto", ...LOCALES];
    const lang = select(langs, cfg.ui.locale, (l) =>
      l === "auto" ? t("settings.ui.languageAuto") : LOCALE_NAMES[l],
    );
    lang.addEventListener("change", () =>
      patch((c) => ({ ...c, ui: { ...c.ui, locale: lang.value as LocalePref } })),
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
      row(t("settings.ui.language"), lang, t("settings.ui.languageHint")),
      row(t("settings.ui.followTheme"), follow),
      row(t("settings.ui.sidebarPreview"), prev),
      row(t("settings.ui.webgl"), webgl, t("settings.ui.webglHint")),
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
        say(t("settings.io.parseFail", { source, error: (err as Error).message }), "warn");
        return;
      }
      host.set(result.config);
      ta.value = exportConfigJson(result.config);
      if (result.warnings.length === 0) {
        say(t("settings.io.ok", { source }), "ok");
      } else {
        say(
          t("settings.io.okWarn", { source, n: result.warnings.length }) +
            "\n· " +
            result.warnings.join("\n· "),
          "warn",
        );
      }
    };

    const applyBtn = el("button", undefined, t("settings.io.apply"));
    applyBtn.addEventListener("click", () => apply(ta.value, t("settings.io.srcImport")));

    const download = el("button", undefined, t("settings.io.download"));
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
      apply(await f.text(), t("settings.io.srcFile"));
      file.value = "";
    });
    const upload = el("button", undefined, t("settings.io.upload"));
    upload.addEventListener("click", () => file.click());

    const reset = el("button", "danger", t("settings.io.reset"));
    reset.addEventListener("click", () => {
      if (!confirm(t("settings.io.resetConfirm"))) return;
      // 和首次打开走同一条路:重新读一次系统亮暗,而不是钉死深色
      host.set(systemDefaultConfig());
      ta.value = exportConfigJson(host.get());
      say(t("settings.io.resetDone", { name: host.get().theme.name }), "ok");
    });

    const bar = el("div", "set-bar");
    bar.append(applyBtn, download, upload, reset, file);
    wrap.append(
      el("div", "set-note", t("settings.io.note")),
      ta,
      bar,
      status,
    );
    return wrap;
  }

  function render() {
    title.textContent = t("app.settings");
    closeBtn.title = t("settings.close");
    renderTabs();
    const pane =
      tab === "theme" ? renderTheme()
      : tab === "font" ? renderFont()
      : tab === "terminal" ? renderTerminal()
      : tab === "ui" ? renderUi()
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

  // 语言是在这个面板里改的,所以改完必须原地重绘一次,不然用户看着的还是旧文案。
  subscribe(() => {
    if (!overlay.hidden) render();
  });

  return {
    open() {
      overlay.hidden = false;
      render();
    },
  };
}
