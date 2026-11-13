// The browser client the share injects into every HTML page: comments,
// the agent's answers, the connection state, live reload, the control
// panel, presence and cursors (docs/ui.md).

import { Answers } from "./answers";
import { AGENT_TARGET_PREFIX, LIVE_REVISION } from "./config";
import { Connection, type ConnectionView } from "./connection";
import { CursorLayer, CursorSender } from "./cursors";
import { ANSWER_LABEL, CommentDialog } from "./dialog";
import { EditMode } from "./edit";
import type { PageSettings, Person } from "./generated/api";
import { CommentList } from "./list";
import { Registration, TAB_ID, pageUrl, watchUrl } from "./location";
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
  const sender = new CursorSender(ui);
  const layer = new CursorLayer(ui);
  const registration = new Registration();
  let reloadRevision = LIVE_REVISION;
  let connectionView: ConnectionView = { colour: "amber", text: "", incompatible: null };
  const editMode = new EditMode(
    ui,
    dialog,
    () => connectionView.incompatible === null && panel.commentMode,
  );
  editMode.onPagePress = () => {
    panel.close();
    list.close();
  };
  let url = pageUrl();
  let you: string | null = null;
  let here: Person[] = [];

  list.onOpen = () => { panel.close(); };
  panel.onOpen = () => { list.close(); };
  outbox.onChange = () => { list.render(); };
  outbox.onQueued = (queued) => {
    // Its count is news unless something newer (its delivery, say) came
    // first; either way, the joiner's views from before it are old now.
    if (connection.fresh(queued.epoch, queued.rev, "mine")) ui.setPending(queued.pending);
    else void connection.refresh();
  };

  const prompt = new NamePrompt(ui, (saved) => {
    if (connection.fresh(saved.epoch, saved.rev, "settings")) panel.show(saved);
  });
  panel.fresh = (saved) => connection.fresh(saved.epoch, saved.rev, "settings");
  const applySettings = (settings: PageSettings): void => {
    sender.setOn(settings.showCursor === "on");
    prompt.show(settings);
    editMode.refresh();
  };
  panel.onSettings = applySettings;

  const showPeople = (people: Person[]): void => {
    here = people;
    panel.showPeople(here, you);
    layer.setPeople(here, url);
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
      // A joiner this page cannot talk to takes no comments.
      editMode.refresh();
      if (view.colour !== "green") layer.clear();
      // The joiner forgets this tab when the stream goes: no cursor until
      // it is registered again.
      if (!connection.streaming) sender.suspend();
      if (recovered) void outbox.resendAll();
    },
    status(status, parts) {
      if (parts.has("connection")) you = status.you?.key ?? null;
      if (parts.has("mine")) {
        ui.setPending(status.pending);
        outbox.markDeliveredExcept(status.undelivered);
      }
      for (const answer of status.answers) answers.add(answer);
      if (parts.has("settings")) panel.show(status.settings);
      // Who is "you" may have changed with the connection: draw again.
      if (parts.has("presence")) showPeople(status.here);
      else if (parts.has("connection")) showPeople(here);
      if (parts.has("reload")) onRevision(status.revision);
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
          if (event.url === url && event.key !== you) layer.update(event.key, event.position);
          break;
        case "share":
        case "status":
        case "ping":
          break;
      }
    },
    opened() {
      register();
    },
  }, () => ({ tab: TAB_ID, url, seq: registration.latest }));

  /**
   * Tells the joiner where this tab is now; cursors go out only once it
   * has taken that page (movement meanwhile waits), and never while the
   * page cannot say (`null`).
   */
  function register(): void {
    if (url === null) sender.suspend();
    else sender.registering();
    registration.request(url);
  }
  registration.onConfirmed = (confirmed, seq) => {
    if (confirmed !== null && confirmed === url && connection.streaming) {
      sender.confirmed(confirmed, seq);
    }
  };

  // Single-page apps move without loading: follow the URL, tell the joiner.
  watchUrl((now) => {
    url = now;
    layer.clear();
    layer.setPeople(here, url);
    if (connection.streaming) register();
    else sender.suspend();
  });
  // The tab in front is where the joiner is.
  const activate = (): void => {
    if (connection.streaming && document.visibilityState === "visible") register();
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
