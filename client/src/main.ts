// The browser client the share injects into every HTML page: comments,
// the agent's answers, the connection state, live reload, the control
// panel, presence and cursors (docs/ui.md).

import { Answers } from "./answers";
import { AGENT_TARGET_PREFIX, EDIT_REF_KEY, LIVE_REVISION } from "./config";
import { Connection } from "./connection";
import { CursorLayer, CursorSender } from "./cursors";
import { CommentDialog } from "./dialog";
import { loadDraft } from "./draft";
import { EditMode } from "./edit";
import type { PageSettings, Person, Snapshot } from "./generated/api";
import { CommentList } from "./list";
import { isThisPage, pageUrl, watchUrl } from "./location";
import { NamePrompt } from "./name";
import { Outbox } from "./outbox";
import { type ChangeSettings, Panel, isEditing } from "./panel";
import { forget, keep, missing, remember, remembered } from "./remembered";
import { PageSocket, type Slice } from "./socket";
import type { PdfViewer } from "./pdf-viewer";
import { Ui } from "./ui";

export function install(pdf?: PdfViewer): void {
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
  const outbox = new Outbox(async (submission) => {
    const outcome = await socket.comment(submission);
    if (submission.choice !== undefined && "refused" in outcome &&
      outcome.refused.choiceRefused === true) {
      answers.remove(submission.selector.slice(AGENT_TARGET_PREFIX.length));
    }
    return outcome;
  }, () => connection.isReachable);
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
  const answers = new Answers(ui, ({ ref, ...draft }) => outbox.submit(draft, ref));
  const dialog = new CommentDialog(ui, outbox);
  const list = new CommentList(ui, outbox);
  // The panel takes its keys before the page and the `/` handler below.
  const panel = new Panel(ui, changeSettings);
  panel.onClearAnswers = () => { answers.clear(); };
  const sender = new CursorSender(
    ui,
    (position) => socket.send({ type: "cursor", position }),
  );
  const layer = new CursorLayer(ui);
  const editMode = pdf === undefined ? new EditMode(ui, dialog) : null;
  pdf?.bind(ui, dialog);
  if (editMode !== null) {
    dialog.onOpen = () => { editMode.clearHover(); };
    editMode.onPagePress = () => {
      panel.close();
      list.close();
    };
  }

  list.onPreview = (entry) => { pdf?.previewComment(entry); };
  list.onEdit = async (entry) => {
    if (!isThisPage(entry.draft.page)) {
      try {
        const target = new URL(entry.draft.page);
        if (target.origin === location.origin) {
          if (target.pathname !== location.pathname || target.search !== location.search) {
            sessionStorage.setItem(EDIT_REF_KEY, entry.ref);
            location.assign(target);
            return;
          }
          // Let the hash route render its target before opening the editor;
          // a route may mount it later, after fetching.
          location.assign(target);
          await new Promise<void>((resolve) => {
            window.addEventListener("hashchange", () => {
              requestAnimationFrame(() => { resolve(); });
            }, { once: true });
          });
          await targetMounted(entry.draft.selector, HASH_ROUTE_WAIT_MS);
        }
      } catch {
        // The original page cannot be opened from this tab.
      }
    }
    if (await dialog.edit(entry, (selector) => isThisPage(entry.draft.page) ? answers.targetFor(selector) : null)) {
      list.close();
    }
  };
  list.onOpen = () => { panel.close(); };
  panel.onOpen = () => { list.close(); };
  outbox.onChange = () => {
    list.render();
    pdf?.showComments(outbox.entries);
    const sending = outbox.entries.filter(
      (entry) => entry.draft.choice === undefined && outbox.shown(entry) === "sending",
    ).length;
    const refused = outbox.entries.filter((entry) => entry.state === "failed").length;
    ui.setPending(pending, sending, refused);
  };

  outbox.onInitialOutcome = (outcome) => {
    if ("commented" in outcome) {
      ui.showToast(`Comment queued · ${pending} pending`);
    } else if ("unsent" in outcome) {
      ui.showToast("Not sent: kept in the page, sent when the connection is back");
    }
  };

  const prompt = new NamePrompt(ui, changeSettings);
  panel.onSettings = (settings: PageSettings): void => {
    answers.setTimeoutSeconds(settings.answerTimeout);
    sender.setOn(settings.showCursor === "on");
    prompt.show(settings);
  };

  const connection = new Connection((view, reached) => {
    list.showState(view);
    if (view.colour !== "green") layer.clear();
    if (!connection.isReachable) pdf?.suspend();
    if (reached) {
      void outbox.resendAll();
      pdf?.ensureLoaded();
    }
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
    outbox.sync(undelivered);
  };

  // Only a newer revision reloads: a hello from a socket that opened
  // again may carry the one this page already shows, or an older one.
  const onRevision = (revision: number | null): void => {
    if (revision === null) return;
    if (pdf !== undefined) {
      pdf.requestRevision(revision);
      if (connection.isReachable) pdf.ensureLoaded();
      return;
    }
    if (shownRevision === null || revision <= shownRevision) return;
    shownRevision = revision;
    dialog.reloadWhenClosed();
  };

  // A draft left by a reload comes back once; one on an answer waits for
  // the answers, which come with the first hello.
  let restored = false;
  let editRef: string | null = null;
  try {
    editRef = sessionStorage.getItem(EDIT_REF_KEY);
    sessionStorage.removeItem(EDIT_REF_KEY);
  } catch {
    // No edit request could have been kept for this navigation.
  }
  const restoreDraft = (): void => {
    if (restored) return;
    restored = true;
    const edit = outbox.entries.find((entry) => entry.ref === editRef && entry.state === "failed" &&
      isThisPage(entry.draft.page));
    if (edit !== undefined) {
      void list.onEdit(edit);
      return;
    }
    void dialog.restore((selector) => answers.targetFor(selector));
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
      // Refused (a jamparty that checks more): forgotten, and the joiner's
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
      case "answer_resolved":
        answers.remove(slice.id);
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
    if (pdf !== undefined) {
      if (pdf.commentOnSelection(event)) event.preventDefault();
      return;
    }
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
    if (!pdf?.commentOnSelection()) dialog.openPage();
  }, true);

  outbox.onChange();
  if (editRef === null && loadDraft()?.target.kind !== "answer") restoreDraft();
  socket.start();
}

/** How long an edit waits for a hash route to mount the comment's target. */
const HASH_ROUTE_WAIT_MS = 2000;

/** Resolves once `selector` matches, or after `timeout` ms. */
function targetMounted(selector: string, timeout: number): Promise<void> {
  const found = (): boolean => {
    try {
      return selector !== "" && document.querySelector(selector) !== null;
    } catch {
      return false;
    }
  };
  // Page comments and answers to the agent have no element to wait for.
  if (selector === "" || selector.startsWith(AGENT_TARGET_PREFIX) || found()) {
    return Promise.resolve();
  }
  return new Promise<void>((resolve) => {
    const done = (): void => {
      observer.disconnect();
      clearTimeout(timer);
      resolve();
    };
    const observer = new MutationObserver(() => { if (found()) done(); });
    observer.observe(document.documentElement, {
      subtree: true, childList: true, attributes: true,
    });
    const timer = window.setTimeout(done, timeout);
  });
}
