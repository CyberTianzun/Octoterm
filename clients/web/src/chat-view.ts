import { type Block, type Message } from "./chat";
import { t } from "./i18n";

function excerpt(text: string): string {
  return text.split(/\r?\n/).find((line) => line.trim())?.trim().slice(0, 160) ?? "";
}

function blockNode(block: Block): HTMLElement {
  if (block.kind === "text") {
    const text = document.createElement("div");
    text.className = "cb-text";
    text.textContent = block.text;
    return text;
  }

  const details = document.createElement("details");
  const summary = document.createElement("summary");
  const label = document.createElement("span");
  label.className = "cb-label";
  const body = document.createElement(block.kind === "thinking" ? "div" : "pre");
  summary.appendChild(label);
  details.append(summary, body);

  if (block.kind === "thinking") {
    details.className = "cb-thinking";
    label.textContent = t("chat.thinking");
    body.textContent = block.text;
  } else {
    details.className = block.kind === "tool-use" ? "cb-tool" : "cb-result";
    const preview = document.createElement("span");
    preview.className = "cb-preview";
    summary.appendChild(preview);
    if (block.kind === "tool-use") {
      label.textContent = block.name;
      preview.textContent = excerpt(block.input);
      body.textContent = block.input;
    } else {
      label.textContent = t("chat.toolResult");
      preview.textContent = excerpt(block.text);
      body.textContent = block.text;
      if (!block.ok) {
        details.classList.add("is-error");
        const error = document.createElement("span");
        error.className = "cb-result-tag";
        error.textContent = t("chat.toolResultError");
        summary.insertBefore(error, preview);
      }
    }
  }
  return details;
}

/** Preserve disclosure state and the reading position across transcript updates. */
export function renderMessages(box: HTMLElement, messages: Message[], scope: string, scrollContainer: HTMLElement = box,
  options: { prepend?: boolean; header?: HTMLElement } = {}): void {
  const sameScope = box.dataset.chatScope === scope;
  const scrollTop = sameScope ? scrollContainer.scrollTop : 0;
  // Measure before replacing children: an empty container always looks like its bottom.
  const atBottom = !sameScope || scrollContainer.scrollHeight - scrollContainer.scrollTop - scrollContainer.clientHeight < 40;
  const anchor = sameScope && options.prepend
    ? Array.from(box.querySelectorAll<HTMLElement>("[data-message-id]")).find((row) =>
      row.getBoundingClientRect().bottom >= scrollContainer.getBoundingClientRect().top)
    : undefined;
  const anchorTop = anchor?.getBoundingClientRect().top;
  const anchorId = anchor?.dataset.messageId;
  const expanded = new Set<string>();
  let focused: string | undefined;
  if (sameScope) {
    box.querySelectorAll<HTMLDetailsElement>("details[data-block-key]").forEach((details) => {
      if (details.open) expanded.add(details.dataset.blockKey!);
      if (details.firstElementChild === document.activeElement) focused = details.dataset.blockKey;
    });
  }

  const content = document.createDocumentFragment();
  if (options.header) content.appendChild(options.header);
  let focusTarget: HTMLElement | null = null;
  for (const message of messages) {
    const row = document.createElement("div");
    row.className = `cmsg c-${message.role}`;
    row.dataset.messageId = message.id;
    message.blocks.forEach((block, index) => {
      const node = blockNode(block);
      if (node instanceof HTMLDetailsElement) {
        const key = JSON.stringify([message.id, index, block.kind]);
        node.dataset.blockKey = key;
        node.open = expanded.has(key);
        if (key === focused) focusTarget = node.firstElementChild as HTMLElement;
      }
      row.appendChild(node);
    });
    content.appendChild(row);
  }
  box.replaceChildren(content);
  box.dataset.chatScope = scope;
  if (focusTarget) (focusTarget as HTMLElement).focus({ preventScroll: true });
  const restored = anchorId === undefined ? undefined : Array.from(box.querySelectorAll<HTMLElement>("[data-message-id]"))
    .find((row) => row.dataset.messageId === anchorId);
  if (options.prepend && sameScope) {
    scrollContainer.scrollTop = scrollTop;
    if (restored && anchorTop !== undefined) scrollContainer.scrollTop += restored.getBoundingClientRect().top - anchorTop;
  } else scrollContainer.scrollTop = atBottom ? scrollContainer.scrollHeight : scrollTop;
}
