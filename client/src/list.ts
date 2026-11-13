// The pill (connection dot and the joiner's undelivered count) and the
// comment list it expands into: the joiner's own comments, newest first,
// with what each points at, when, and its state.

import { AGENT_TARGET_PREFIX } from "./config";
import type { ConnectionView } from "./connection";
import type { Outbox, OutboxEntry } from "./outbox";
import type { Ui } from "./ui";

/** The last step of a selector, for people: `h1`, `#intro`, `page`. */
export function shortSelector(selector: string): string {
  if (selector === "") return "page";
  if (selector.startsWith(AGENT_TARGET_PREFIX)) return "answer";
  const parts = selector.split(" > ");
  return parts[parts.length - 1] ?? selector;
}

/** `just now`, `3 min ago`, `2 h ago`. */
export function relativeTime(at: number, now = Date.now()): string {
  const minutes = Math.floor((now - at) / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
}

function stateText(entry: OutboxEntry): string {
  switch (entry.state) {
    case "unsent":
      return "not sent";
    case "sending":
      return "sending";
    case "queued":
    case "delivered":
      return entry.state;
  }
}

export class CommentList {
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
    const { outboxList, outboxEmpty } = this.ui;
    const now = Date.now();
    const entries = [...this.outbox.entries].sort((first, second) => second.at - first.at);
    outboxList.replaceChildren(...entries.map((entry) => {
      const item = document.createElement("li");
      item.dataset.state = entry.state;
      const row = document.createElement("div");
      row.className = "row";
      const where = document.createElement("span");
      where.className = "where";
      where.textContent = `${shortSelector(entry.draft.selector)} · ${relativeTime(entry.at, now)}`;
      where.title = entry.draft.selector || "the whole page";
      const state = document.createElement("span");
      state.className = "state";
      state.textContent = stateText(entry);
      row.append(where, state);
      const text = document.createElement("p");
      text.className = "text";
      text.textContent = entry.draft.comment;
      item.append(row, text);
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
    outboxEmpty.hidden = entries.length > 0;
  }
}
