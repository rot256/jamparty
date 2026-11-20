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
  private hovered: OutboxEntry | null = null;
  private focused: OutboxEntry | null = null;
  onPreview: (entry: OutboxEntry | null) => void = () => undefined;
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

  render(): void {
    const { outboxList } = this.ui;
    const now = Date.now();
    const entries = [...this.outbox.entries].sort((first, second) => second.at - first.at);
    const previousFocus = this.focused;
    const root = outboxList.getRootNode();
    const active = root instanceof Document || root instanceof ShadowRoot ? root.activeElement : null;
    const againFocused = active instanceof HTMLElement && active.classList.contains("again") &&
      outboxList.contains(active);
    this.hovered = this.focused = null;
    this.onPreview(null);
    outboxList.replaceChildren(...entries.map((entry) => {
      const item = document.createElement("li");
      const shown = this.outbox.shown(entry);
      item.dataset.state = shown;
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
        // Focusable from code only, for when its "send again" button goes
        // away under the focus; no extra Tab stop.
        item.tabIndex = -1;
      }
      // Focus is tracked for every row, so a refresh can restore it. The
      // latest of hover and focus wins, so a keyboard user sees the row
      // they moved to even with the pointer resting on another.
      item.addEventListener("focusin", () => {
        this.focused = entry;
        if (entry.draft.pdf !== undefined) this.onPreview(entry);
      });
      item.addEventListener("focusout", (event) => {
        if (event.relatedTarget instanceof Node && item.contains(event.relatedTarget)) return;
        this.focused = null;
        this.onPreview(this.hovered);
      });
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
      row.append(where, state);
      const text = document.createElement("p");
      text.className = "text";
      text.textContent = entry.draft.comment;
      item.append(row, text);
      for (const attachment of entry.draft.attachments ?? []) {
        const file = document.createElement("p");
        file.className = "text";
        file.textContent = attachment.name;
        item.append(file);
      }
      if (entry.error !== undefined) {
        const error = document.createElement("p");
        error.className = "state";
        error.textContent = entry.error;
        item.append(error);
      }
      if (entry.state === "unsent") {
        const again = document.createElement("button");
        again.type = "button";
        again.className = "again";
        again.textContent = "send again";
        again.addEventListener("click", () => { void this.outbox.resend(entry); });
        item.append(again);
      }
      return item;
    }));
    if (previousFocus !== null) {
      const index = entries.indexOf(previousFocus);
      const item = outboxList.children.item(index);
      // Keep focus on "send again" while the comment can still be sent again.
      const again = againFocused ? item?.querySelector<HTMLElement>(".again") : null;
      if (again instanceof HTMLElement) again.focus({ preventScroll: true });
      else if (item instanceof HTMLElement) item.focus({ preventScroll: true });
    }
  }
}
