// The browser client the share injects into every HTML page: comments,
// the agent's answers, the connection state, live reload, the control
// panel, presence and cursors (docs/ui.md).

import { Answers } from "./answers";
import { AGENT_TARGET_PREFIX, LIVE_REVISION } from "./config";
import { Connection, type ConnectionView } from "./connection";
import { CursorLayer, CursorSender } from "./cursors";
import { CommentDialog } from "./dialog";
import { loadDraft } from "./draft";
import { EditMode } from "./edit";
import type { Hello, Outcome, PageSettings, Person, SettingsChange } from "./generated/api";
import { CommentList } from "./list";
import { pageUrl, watchUrl } from "./location";
import { NamePrompt } from "./name";
import { Outbox } from "./outbox";
import { Panel, isEditing } from "./panel";
import { PageSocket, type Slice } from "./socket";
import { Ui } from "./ui";

function install(): void {
  const ui = Ui.install();
  if (ui === null) return;
  let url = pageUrl();
  let you: string | null = null;
  let here: Person[] = [];
  /** How many of this joiner's comments the share holds, as last said. */
  let pending = 0;
  let reloadRevision = LIVE_REVISION;
  let connectionView: ConnectionView = { colour: "amber", text: "", incompatible: null };

  // Everything the page asks of the joiner goes over its one socket.
  const socket = new PageSocket({
    hello(hello) { applyHello(hello); },
    slice(slice) { applySlice(slice); },
    open(open) {
      connection.setOpen(open);
      if (open) report();
      else sender.setPlaced(false);
    },
    incompatible(reason) { connection.refuse(reason); },
  });
  const outbox = new Outbox((submission) => socket.request({ type: "comment", ...submission }));
  const changeSettings = (change: SettingsChange): Promise<Outcome> =>
    socket.request({ type: "settings", change });
  const answers = new Answers(ui);
  const dialog = new CommentDialog(ui, outbox, () => pending);
  const list = new CommentList(ui, outbox);
  // The panel takes its keys before the page and the `/` handler below.
  const panel = new Panel(ui, changeSettings);
  const sender = new CursorSender(
    ui,
    (position) => socket.send({ type: "cursor", position }),
  );
  const layer = new CursorLayer(ui);
  const editMode = new EditMode(
    ui,
    dialog,
    () => connectionView.incompatible === null && panel.commentMode,
  );
  editMode.onPagePress = () => {
    panel.close();
    list.close();
  };

  list.onOpen = () => { panel.close(); };
  panel.onOpen = () => { list.close(); };
  outbox.onChange = () => { list.render(); };

  const prompt = new NamePrompt(ui, changeSettings);
  panel.onSettings = (settings: PageSettings): void => {
    sender.setOn(settings.showCursor === "on");
    prompt.show(settings);
    editMode.refresh();
  };

  const connection = new Connection((view, recovered) => {
    connectionView = view;
    list.showState(view);
    ui.notice.hidden = view.incompatible === null;
    ui.notice.textContent = view.incompatible ?? "";
    // A joiner this page cannot talk to takes no comments.
    editMode.refresh();
    if (view.colour !== "green") layer.clear();
    if (recovered) void outbox.resendAll();
  });

  const showPeople = (people: Person[]): void => {
    here = people;
    panel.showPeople(here, you);
    layer.setPeople(here, url);
  };

  const showMine = (undelivered: string[] | null): void => {
    // Unknown (a joiner that restarted, before the share said): as it was.
    if (undelivered === null) return;
    pending = undelivered.length;
    ui.setPending(pending);
    outbox.sync(undelivered);
  };

  const onRevision = (revision: number | null): void => {
    if (LIVE_REVISION === null || revision === null) return;
    if (String(revision) === reloadRevision) return;
    reloadRevision = String(revision);
    dialog.reloadWhenClosed();
  };

  // A draft left by a reload comes back once; one on an answer waits for
  // the answers, which come with the first hello.
  let restored = false;
  const restoreDraft = (): void => {
    if (restored) return;
    restored = true;
    dialog.restore((selector) => answers.targetFor(selector));
  };

  function applyHello(hello: Hello): void {
    you = hello.link.you?.key ?? null;
    showMine(hello.mine);
    for (const answer of hello.answers) answers.add(answer);
    restoreDraft();
    panel.show(hello.settings);
    showPeople(hello.people);
    onRevision(hello.revision);
    connection.setLink(hello.link);
  }

  function applySlice(slice: Slice): void {
    switch (slice.type) {
      case "link":
        you = slice.you?.key ?? null;
        // Who is "you" may have changed: draw again.
        showPeople(here);
        connection.setLink(slice);
        break;
      case "people":
        showPeople(slice.here);
        break;
      case "mine":
        showMine(slice.undelivered);
        break;
      case "settings":
        panel.show(slice);
        break;
      case "reload":
        onRevision(slice.revision);
        break;
      case "answer":
        answers.add(slice);
        break;
      case "cursor":
        if (slice.url === url && slice.key !== you) layer.update(slice.key, slice.position);
        break;
    }
  }

  /**
   * Tells the joiner this tab is here and in front. Pointers go out on the
   * same socket after it, so the joiner reads them on this page; none
   * while the page cannot say where it is.
   */
  function report(): void {
    if (!socket.send({ type: "page", url })) return;
    sender.setPlaced(url !== null);
  }

  // Single-page apps move without loading: follow the URL, tell the joiner.
  watchUrl((now) => {
    url = now;
    layer.clear();
    layer.setPeople(here, url);
    report();
  });
  // The tab in front is where the joiner is.
  const activate = (): void => {
    if (document.visibilityState === "visible") report();
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
  if (loadDraft()?.target.kind !== "answer") restoreDraft();
  socket.start();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", install, { once: true });
} else {
  install();
}
