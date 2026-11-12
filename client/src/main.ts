// The browser client the share injects into every HTML page: comments,
// the agent's answers, the connection state, live reload, the control
// panel and presence (docs/ui.md).

import { Answers } from "./answers";
import { AGENT_TARGET_PREFIX, LIVE_REVISION } from "./config";
import { Connection, type ConnectionView } from "./connection";
import { ANSWER_LABEL, CommentDialog } from "./dialog";
import type { PageSettings, Person } from "./generated/api";
import { CommentList } from "./list";
import { TAB_ID, pageUrl, reportUrl, watchUrl } from "./location";
import { NamePrompt } from "./name";
import { Outbox } from "./outbox";
import { Panel, isEditing } from "./panel";
import { Ui } from "./ui";

function install(): void {
  const ui = Ui.install();
  if (ui === null) return;
  const outbox = new Outbox();
  const answers = new Answers(ui);
  const dialog = new CommentDialog(ui, outbox);
  const list = new CommentList(ui, outbox);
  // The panel takes its keys before the page and the `/` handler below.
  const panel = new Panel(ui);
  let reloadRevision = LIVE_REVISION;
  let connectionView: ConnectionView = { colour: "amber", text: "", incompatible: null };
  let url = pageUrl();
  let you: string | null = null;
  let here: Person[] = [];

  list.onOpen = () => { panel.close(); };
  panel.onOpen = () => { list.close(); };
  outbox.onChange = () => { list.render(); };
  outbox.onPending = (pending) => { ui.setPending(pending); };

  const prompt = new NamePrompt(ui, (settings) => { panel.show(settings); });
  const applySettings = (settings: PageSettings): void => {
    prompt.show(settings);
  };
  panel.onSettings = applySettings;

  const showPeople = (people: Person[]): void => {
    here = people;
    panel.showPeople(here, you);
  };

  const onRevision = (revision: number | null): void => {
    if (LIVE_REVISION === null || revision === null) return;
    if (String(revision) === reloadRevision) return;
    reloadRevision = String(revision);
    dialog.reloadWhenClosed();
  };

  const connection = new Connection({
    state(view, recovered) {
      connectionView = view;
      list.showState(view);
      ui.notice.hidden = view.incompatible === null;
      ui.notice.textContent = view.incompatible ?? "";
      if (recovered) void outbox.resendAll();
    },
    status(status) {
      you = status.you?.key ?? null;
      ui.setPending(status.pending);
      outbox.markDeliveredExcept(status.undelivered);
      for (const answer of status.answers) answers.add(answer);
      panel.show(status.settings);
      showPeople(status.here);
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
        case "presence":
          showPeople(event.here);
          break;
        case "settings":
          panel.show(event);
          break;
        case "cursor":
        case "share":
        case "status":
        case "ping":
          break;
      }
    },
  }, () => (url === null ? null : { tab: TAB_ID, url }));

  // Single-page apps move without loading: follow the URL, tell the joiner.
  watchUrl((now) => {
    url = now;
    if (connection.streaming) reportUrl(url);
  });
  // The tab in front is where the joiner is.
  const activate = (): void => {
    if (connection.streaming && document.visibilityState === "visible") reportUrl(url);
  };
  window.addEventListener("focus", activate);
  document.addEventListener("visibilitychange", activate);

  const commenting = (): boolean => connectionView.incompatible === null && panel.commentMode;

  document.addEventListener("contextmenu", (event) => {
    if (!commenting()) return;
    if (event.shiftKey || event.composedPath().includes(ui.host)) return;
    if (!(event.target instanceof Element)) return;
    event.preventDefault();
    dialog.openElement(event.target, event.clientX, event.clientY);
  }, true);

  ui.shadow.addEventListener("contextmenu", (event) => {
    if (!commenting()) return;
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
    if (!commenting()) return;
    if (
      event.defaultPrevented || event.isComposing || event.key !== "/" ||
      event.metaKey || event.ctrlKey || event.altKey
    ) return;
    if (isEditing(event)) return;
    event.preventDefault();
    dialog.openPage();
  }, true);

  list.render();
  void connection.start();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", install, { once: true });
} else {
  install();
}
