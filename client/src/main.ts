// The browser client the share injects into every HTML page: comments,
// the agent's answers, the connection state and live reload.

import { Answers } from "./answers";
import { AGENT_TARGET_PREFIX, LIVE_REVISION } from "./config";
import { Connection, type ConnectionView } from "./connection";
import { ANSWER_LABEL, CommentDialog } from "./dialog";
import { Outbox, type OutboxEntry } from "./outbox";
import { Ui } from "./ui";

function stateText(entry: OutboxEntry): string {
  return entry.state === "unsent" ? "not sent" : entry.state;
}

function isEditing(event: Event): boolean {
  return event.composedPath().some(
    (node) => node instanceof HTMLElement &&
      (node.isContentEditable || node.matches("input, textarea, select")),
  );
}

function install(): void {
  const ui = Ui.install();
  if (ui === null) return;
  const outbox = new Outbox();
  const answers = new Answers(ui);
  const dialog = new CommentDialog(ui, outbox);
  let reloadRevision = LIVE_REVISION;
  let connectionView: ConnectionView = { colour: "amber", text: "", incompatible: null };

  const renderOutbox = (): void => {
    ui.outboxList.replaceChildren(...outbox.entries.map((entry) => {
      const item = document.createElement("li");
      item.dataset.state = entry.state;
      const text = document.createElement("span");
      text.className = "text";
      text.textContent = entry.draft.comment;
      const state = document.createElement("span");
      state.className = "state";
      state.textContent = stateText(entry);
      item.append(text, state);
      if (entry.state === "unsent") {
        const again = document.createElement("button");
        again.type = "button";
        again.textContent = "send again";
        again.addEventListener("click", () => { void outbox.resend(entry); });
        item.append(again);
      }
      return item;
    }));
    ui.outbox.hidden = outbox.entries.length === 0 && ui.connectionText.hidden;
  };
  outbox.onChange = renderOutbox;
  outbox.onPending = (pending) => { ui.setPending(pending); };

  const onRevision = (revision: number | null): void => {
    if (LIVE_REVISION === null || revision === null) return;
    if (String(revision) === reloadRevision) return;
    reloadRevision = String(revision);
    dialog.reloadWhenClosed();
  };

  const connection = new Connection({
    state(view, recovered) {
      connectionView = view;
      ui.connection.dataset.state = view.colour;
      ui.connection.title = view.text;
      ui.connection.setAttribute("aria-label", view.text);
      ui.connectionText.textContent = view.text;
      ui.connectionText.hidden = view.colour === "green";
      renderOutbox();
      if (recovered) void outbox.resendAll();
    },
    status(status) {
      ui.setPending(status.pending);
      outbox.markDeliveredExcept(status.undelivered);
      for (const answer of status.answers) answers.add(answer);
      onRevision(status.revision);
    },
    event(event) {
      switch (event.type) {
        case "answer":
          answers.add(event);
          break;
        case "reload":
          onRevision(event.revision);
          break;
        case "delivered":
          outbox.markDelivered(event.ids);
          break;
        case "share":
        case "status":
        case "presence":
        case "ping":
          break;
      }
    },
  });

  document.addEventListener("contextmenu", (event) => {
    if (connectionView.incompatible !== null) return;
    if (event.shiftKey || event.composedPath().includes(ui.host)) return;
    if (!(event.target instanceof Element)) return;
    event.preventDefault();
    dialog.openElement(event.target, event.clientX, event.clientY);
  }, true);

  ui.shadow.addEventListener("contextmenu", (event) => {
    if (connectionView.incompatible !== null) return;
    if (!(event instanceof MouseEvent) || event.shiftKey) return;
    const target = event.composedPath().find(
      (node) => node instanceof HTMLElement && node.classList.contains("agent-message"),
    );
    if (!(target instanceof HTMLElement) || !target.dataset.messageId) return;
    event.preventDefault();
    event.stopPropagation();
    dialog.openElement(
      target,
      event.clientX,
      event.clientY,
      `${AGENT_TARGET_PREFIX}${target.dataset.messageId}`,
      ANSWER_LABEL,
    );
  }, true);

  document.addEventListener("keydown", (event) => {
    if (connectionView.incompatible !== null) return;
    if (
      event.defaultPrevented || event.isComposing || event.key !== "/" ||
      event.metaKey || event.ctrlKey || event.altKey
    ) return;
    if (isEditing(event)) return;
    event.preventDefault();
    dialog.openPage();
  }, true);

  renderOutbox();
  void connection.start();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", install, { once: true });
} else {
  install();
}
