// The pill (connection dot and the joiner's undelivered count) and the
// comment list it expands into: the joiner's own comments, newest first,
// with what each points at, when, and its state.

import { AGENT_TARGET_PREFIX } from "./config";
import type { ConnectionView } from "./connection";
import type { Outbox, OutboxEntry, ShownState } from "./outbox";
import type { Ui } from "./ui";

/** The last step of a selector, for people: `h1`, `#intro`, `page`. */
function shortSelector(selector: string): string {
  if (selector === "") return "page";
  if (selector.startsWith(AGENT_TARGET_PREFIX)) return "answer";
  const parts = selector.split(" > ");
  return parts[parts.length - 1] ?? selector;
}

/** `just now`, `3 min ago`, `2 h ago`. */
function relativeTime(at: number, now = Date.now()): string {
  const minutes = Math.floor((now - at) / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
}

function stateText(state: ShownState): string {
  switch (state) {
    case "failed":
      return "refused";
    case "unsent":
      return "not sent";
    case "sending":
      return "sending";
    case "earlier":
      // Queued under an earlier jamparty join; this one cannot follow it.
      return "sent earlier";
    case "queued":
    case "delivered":
      return state;
  }
}

export class CommentList {
  private readonly items = new Map<OutboxEntry, HTMLLIElement>();
  private hovered: OutboxEntry | null = null;
  private focused: OutboxEntry | null = null;
  onPreview: (entry: OutboxEntry | null) => void = () => undefined;
  onEdit: (entry: OutboxEntry) => Promise<void> = () => Promise.resolve();
  private refreshTimer: number | undefined;
  /** Called when the list opens (the control panel closes then). */
  onOpen: () => void = () => undefined;

  constructor(
    private readonly ui: Ui,
    private readonly outbox: Outbox,
  ) {
    ui.pill.addEventListener("click", () => { this.toggle(); });
    ui.listHeader.addEventListener("click", () => { this.close(); });
    // A press anywhere outside the open list closes it; the press itself
    // still does what it does.
    document.addEventListener("pointerdown", (event) => {
      if (this.isOpen && !event.composedPath().includes(ui.list)) this.close();
    }, true);
  }

  get isOpen(): boolean {
    return !this.ui.list.hidden;
  }

  toggle(): void {
    if (this.isOpen) this.close();
    else this.open();
  }

  open(): void {
    this.onOpen();
    this.ui.list.hidden = false;
    this.ui.bar.hidden = true;
    this.ui.pill.setAttribute("aria-expanded", "true");
    this.render();
    // Relative times move on while the list is open.
    this.refreshTimer = window.setInterval(() => { this.render(); }, 30000);
    this.ui.collapse.focus();
  }

  close(): void {
    if (!this.isOpen) return;
    this.ui.list.hidden = true;
    this.hovered = this.focused = null;
    this.onPreview(null);
    this.ui.bar.hidden = false;
    this.ui.pill.setAttribute("aria-expanded", "false");
    if (this.refreshTimer !== undefined) clearInterval(this.refreshTimer);
    this.refreshTimer = undefined;
  }

  /** Shows the connection state in the pill and the list's header. */
  showState(view: ConnectionView): void {
    const { connection, listDot, connectionText, pill } = this.ui;
    connection.dataset.state = view.colour;
    listDot.dataset.state = view.colour;
    connection.title = view.text;
    connection.setAttribute("aria-label", view.text);
    pill.title = view.text;
    connectionText.dataset.state = view.colour;
    connectionText.textContent = `· ${view.text}`;
  }

  private createItem(entry: OutboxEntry): HTMLLIElement {
    const item = document.createElement("li");
    if (entry.draft.pdf !== undefined) {
      item.tabIndex = 0;
      item.addEventListener("pointerenter", () => {
        this.hovered = entry;
        this.onPreview(entry);
      });
      item.addEventListener("pointerleave", () => {
        this.hovered = null;
        this.onPreview(this.focused?.draft.pdf === undefined ? null : this.focused);
      });
    } else {
      // Focusable from code when an action goes away, without an extra Tab stop.
      item.tabIndex = -1;
    }
    item.addEventListener("focusin", () => {
      this.focused = entry;
      if (entry.draft.pdf !== undefined) this.onPreview(entry);
    });
    item.addEventListener("focusout", (event) => {
      if (event.relatedTarget instanceof Node && item.contains(event.relatedTarget)) return;
      this.focused = null;
      this.onPreview(this.hovered);
    });
    this.items.set(entry, item);
    return item;
  }

  render(): void {
    const { outboxList } = this.ui;
    const now = Date.now();
    const entries = [...this.outbox.entries].sort((first, second) => second.at - first.at);
    const active = this.ui.shadow.activeElement;
    const focusedItem = active?.closest("li");
    const focusedAction = active instanceof HTMLButtonElement ? active.textContent : null;
    for (const [entry, item] of this.items) {
      if (entries.includes(entry)) continue;
      item.remove();
      this.items.delete(entry);
      if (this.hovered === entry || this.focused === entry) {
        if (this.hovered === entry) this.hovered = null;
        if (this.focused === entry) this.focused = null;
        this.onPreview(this.hovered ?? (this.focused?.draft.pdf === undefined ? null : this.focused));
      }
    }
    for (const [index, entry] of entries.entries()) {
      const item = this.items.get(entry) ?? this.createItem(entry);
      const atIndex = outboxList.children.item(index);
      if (atIndex !== item) outboxList.insertBefore(item, atIndex);
      const shown = this.outbox.shown(entry);
      item.dataset.state = shown;
      const row = document.createElement("div");
      row.className = "row";
      const where = document.createElement("span");
      where.className = "where";
      const target = entry.draft.pdf === undefined
        ? shortSelector(entry.draft.selector) : `PDF page ${entry.draft.pdf.page}`;
      where.textContent = `${target} · ${relativeTime(entry.at, now)}`;
      where.title = entry.draft.pdf === undefined ? entry.draft.selector || "the whole page" : target;
      const state = document.createElement("span");
      state.className = "state";
      state.textContent = stateText(shown);
      if (shown === "sending") {
        const spinner = document.createElement("span");
        spinner.className = "sending-indicator";
        spinner.setAttribute("aria-hidden", "true");
        state.prepend(spinner);
      }
      row.append(where, state);
      const text = document.createElement("p");
      text.className = "text";
      text.textContent = entry.draft.comment;
      item.replaceChildren(row, text);
      const names = (entry.draft.attachments ?? []).map((file) => file.name);
      if (entry.uploaded !== true) names.push(...(entry.files ?? []));
      for (const name of names) {
        const file = document.createElement("p");
        file.className = "text";
        file.textContent = name;
        item.append(file);
      }
      if (entry.error !== undefined) {
        const error = document.createElement("p");
        error.className = "state";
        error.textContent = entry.error;
        item.append(error);
      }
      if (entry.state === "failed") {
        const edit = document.createElement("button");
        edit.type = "button";
        edit.className = "again";
        edit.textContent = "edit";
        edit.addEventListener("click", () => { void this.onEdit(entry); });
        const discard = document.createElement("button");
        discard.type = "button";
        discard.className = "again";
        discard.textContent = "discard";
        discard.addEventListener("click", () => { this.outbox.discard(entry); });
        item.append(edit, discard);
      }
      if (shown !== "sending" && this.outbox.canResend(entry)) {
        const again = document.createElement("button");
        again.type = "button";
        again.className = "again";
        again.textContent = "send again";
        again.addEventListener("click", () => { void this.outbox.resend(entry); });
        item.append(again);
      }
      if (item === focusedItem) {
        const button = focusedAction === null ? undefined : Array.from(item.querySelectorAll("button"))
          .find((candidate) => candidate.textContent === focusedAction);
        (button ?? item).focus({ preventScroll: true });
      }
    }
  }
}
