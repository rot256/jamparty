// The browser client the share injects into every HTML page: comments,
// the agent's answers, the connection state, live reload, the control
// panel, presence and cursors (docs/ui.md).

import { Answers } from "./answers";
import { AGENT_TARGET_PREFIX, LIVE_REVISION } from "./config";
import { Connection } from "./connection";
import { CursorLayer, CursorSender } from "./cursors";
import { CommentDialog } from "./dialog";
import { loadDraft } from "./draft";
import { EditMode } from "./edit";
import type { PageSettings, Person, Snapshot } from "./generated/api";
import { CommentList } from "./list";
import { pageUrl, watchUrl } from "./location";
import { NamePrompt } from "./name";
import { Outbox } from "./outbox";
import { type ChangeSettings, Panel, isEditing } from "./panel";
import { forget, keep, missing, remember, remembered } from "./remembered";
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
  /** The live-reload revision this page shows (shared directories only). */
  let shownRevision = LIVE_REVISION;

  // Everything the page asks of the joiner goes over its one socket.
  const socket = new PageSocket({
    hello(hello) { applyHello(hello); },
    slice(slice) { applySlice(slice); },
    open(open) {
      connection.setOpen(open);
      if (open) report();
      else sender.setPlaced(false);
    },
  });
  const outbox = new Outbox((submission) => socket.comment(submission));
  // The joiner takes a change for this join only; the page keeps it for
  // the share, from before it goes (the tab may close before the reply),
  // and takes it back if the joiner refuses it.
  const changeSettings: ChangeSettings = async (change) => {
    const before = remembered();
    remember(change);
    const outcome = await socket.changeSettings(change);
    if (outcome !== "saved" && socket.isOpen) keep(before);
    return outcome;
  };
  const answers = new Answers(ui);
  const dialog = new CommentDialog(ui, outbox, () => pending);
  const list = new CommentList(ui, outbox);
  // The panel takes its keys before the page and the `/` handler below.
  const panel = new Panel(ui, changeSettings);
  panel.onClearAnswers = () => { answers.clear(); };
  const sender = new CursorSender(
    ui,
    (position) => socket.send({ type: "cursor", position }),
  );
  const layer = new CursorLayer(ui);
  const editMode = new EditMode(
    ui,
    dialog,
    () => panel.commentMode,
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
    answers.setTimeoutSeconds(settings.answerTimeout);
    sender.setOn(settings.showCursor === "on");
    prompt.show(settings);
    editMode.refresh();
  };

  const connection = new Connection((view, reached) => {
    list.showState(view);
    if (view.colour !== "green") layer.clear();
    if (reached) void outbox.resendAll();
  });

  const showPeople = (people: Person[]): void => {
    here = people;
    panel.showPeople(here, you);
    layer.setPeople(here, url);
  };

  const showMine = (undelivered: string[] | null): void => {
    // Unknown (before the share first admitted the joiner): as it was.
    if (undelivered === null) return;
    pending = undelivered.length;
    ui.setPending(pending);
    outbox.sync(undelivered);
  };

  // Only a newer revision reloads: a hello from a socket that opened
  // again may carry the one this page already shows, or an older one.
  const onRevision = (revision: number | null): void => {
    if (shownRevision === null || revision === null || revision <= shownRevision) return;
    shownRevision = revision;
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

  function applyHello(hello: Snapshot): void {
    you = hello.link.you?.key ?? null;
    outbox.setKey(you);
    showMine(hello.mine);
    answers.keepDismissed(hello.answers);
    for (const answer of hello.answers) answers.add(answer);
    restoreDraft();
    showSettings(hello.settings);
    showPeople(hello.people);
    onRevision(hello.revision);
    connection.setLink(hello.link);
  }

  /**
   * Shows the joiner's settings, after handing it what this share's
   * pages changed if it does not have that yet (a joiner that started
   * over): a remembered name then keeps the name prompt away.
   */
  function showSettings(settings: PageSettings): void {
    const change = missing(remembered(), settings);
    if (change === null) {
      panel.show(settings);
      return;
    }
    panel.show({ ...settings, askName: settings.askName && change.name === undefined });
    void socket.changeSettings(change).then((outcome) => {
      // Taken: the settings message before the reply showed it. Lost
      // with the socket: the next hello tries again.
      if (outcome === "saved" || !socket.isOpen) return;
      // Refused (a jampot that checks more): forgotten, and the joiner's
      // settings shown as they are.
      forget();
      panel.show(settings);
    });
  }

  function applySlice(slice: Slice): void {
    switch (slice.type) {
      case "link":
        you = slice.you?.key ?? null;
        outbox.setKey(you);
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

  const commenting = (): boolean => panel.commentMode;

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
