// The overlay's elements, in a shadow root of their own: the pill with
// the connection dot and the gear (bottom right), the comment list the
// pill expands into, the control panel, other people's cursors, the
// agent's answers and the dialogs.

import { HOST_ID } from "./config";
import { DEFAULT_ANSWER_TIMEOUT, DEFAULT_CLEAR_ANSWERS_KEY, MAX_ANSWER_TIMEOUT, MAX_COMMENT, MAX_NAME } from "./generated/api";
import { styles } from "./styles";

const GEAR_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"></path></svg>`;

const PEN_ICON = `<svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 3 5 5M3 21l5-1L21 7a2.1 2.1 0 0 0-5-5L3 15z"></path></svg>`;

const PAPERCLIP_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21 11-9 9a5 5 0 0 1-7-7l10-10a3.5 3.5 0 0 1 5 5L10 18a1.5 1.5 0 0 1-2-2l9-9"></path></svg>`;

const MARKUP = `
  <style>${styles}</style>
  <div id="cursors" aria-hidden="true"></div>
  <div id="highlight" aria-hidden="true"></div>
  <div id="edit-highlight" aria-hidden="true"></div>
  <div id="message-highlight" aria-hidden="true"></div>
  <div id="bar">
    <button id="pill" type="button" aria-expanded="false" aria-controls="list">
      <span id="connection" class="dot" data-state="amber" role="img" aria-label="Connecting" title="Connecting"></span>
      <span id="badge" role="status" aria-live="polite">0 queued</span>
    </button>
    <button id="edit" type="button" aria-label="Edit mode" aria-pressed="false" title="Edit mode: click an element to comment on it (Esc leaves)">${PEN_ICON}</button>
    <button id="gear" type="button" aria-label="Settings" aria-expanded="false" aria-controls="panel">${GEAR_ICON}</button>
  </div>
  <section id="list" aria-label="Your comments" hidden>
    <header id="list-header">
      <div id="list-summary">
        <span id="list-dot" class="dot" data-state="amber"></span>
        <span id="list-count">0 queued</span>
        <span id="connection-text" role="status">· connecting</span>
      </div>
      <button id="collapse" type="button" aria-label="Collapse">▾</button>
    </header>
    <ol id="outbox-list"></ol>
  </section>
  <section id="panel" aria-label="jamparty settings" hidden>
    <div class="control" id="comment-mode-row">
      <span class="label" id="comment-mode-label">Comment mode</span>
      <button class="chip" id="comment-mode-key" type="button" aria-label="Key for comment mode">C</button>
      <button class="switch" id="comment-mode" type="button" role="switch" aria-checked="true" aria-labelledby="comment-mode-label"></button>
    </div>
    <div class="control" id="show-cursor-row">
      <span class="label" id="show-cursor-label">Show my cursor</span>
      <button class="chip" id="show-cursor-key" type="button" aria-label="Key for show my cursor">X</button>
      <button class="switch" id="show-cursor" type="button" role="switch" aria-checked="false" aria-labelledby="show-cursor-label"></button>
    </div>
    <div class="control">
      <span class="label">Clear answers</span>
      <button class="chip" id="clear-answers-key" type="button" aria-label="Key for clear answers">${DEFAULT_CLEAR_ANSWERS_KEY.toUpperCase()}</button>
      <button id="clear-answers" type="button">Clear</button>
    </div>
    <div class="control">
      <label class="label" for="answer-timeout">Answer timeout (s; 0 = off)</label>
      <input id="answer-timeout" type="number" required min="0" max="${MAX_ANSWER_TIMEOUT}" step="1" value="${DEFAULT_ANSWER_TIMEOUT}">
    </div>
    <div class="control" id="corner-row">
      <span class="label" id="corner-label">Corner</span>
      <span id="corners" role="group" aria-labelledby="corner-label">
        <button class="corner" type="button" data-corner="top-left" aria-label="Top left" aria-pressed="false">↖</button>
        <button class="corner" type="button" data-corner="top-right" aria-label="Top right" aria-pressed="false">↗</button>
        <button class="corner" type="button" data-corner="bottom-left" aria-label="Bottom left" aria-pressed="false">↙</button>
        <button class="corner" type="button" data-corner="bottom-right" aria-label="Bottom right" aria-pressed="true">↘</button>
      </span>
    </div>
    <div id="name-row">
      <label for="name-field">Name</label>
      <input id="name-field" type="text" maxlength="${MAX_NAME}" autocomplete="nickname" spellcheck="false">
    </div>
    <div id="panel-message" role="status"></div>
    <div id="people">
      <span id="here-count">0 here</span>
      <ul id="avatars" aria-label="People here"></ul>
    </div>
  </section>
  <div id="toast" role="status" aria-live="polite" hidden></div>
  <div id="agent-log" role="log" aria-live="polite" aria-relevant="additions">
    <div id="agent-general"></div>
    <div id="agent-anchored"></div>
  </div>
  <dialog id="jamparty-dialog" aria-labelledby="dialog-label">
    <form>
      <label id="dialog-label" for="comment">Comment:</label>
      <div id="selection-preview" hidden>
        <div id="selection-label">Selected text</div>
        <blockquote id="selection-text"></blockquote>
      </div>
      <textarea id="comment" name="comment" maxlength="${MAX_COMMENT}" required></textarea>
      <ul id="attachments" aria-label="Attachments" hidden></ul>
      <input id="attachment-input" type="file" multiple hidden>
      <div id="missing-attachments" hidden>
        Files from before the reload need to be attached again.
        <button id="discard-missing" type="button">Continue without missing files</button>
      </div>
      <div id="status" role="alert"></div>
      <div id="reload-notice" role="status" hidden>
        Files changed. Reloading when this edit closes.
      </div>
      <div class="actions">
        <button id="attach" type="button" aria-label="Attach files" title="Attach files">${PAPERCLIP_ICON}</button>
        <button id="cancel" type="button">Cancel</button>
        <button id="send" type="submit">Queue comment</button>
      </div>
    </form>
  </dialog>
  <dialog id="name-dialog" aria-labelledby="name-heading">
    <form id="name-form">
      <div id="name-title">
        <strong id="name-heading">Join the Jam.</strong>
        <span>Your name is shown next to your comments and your cursor.</span>
      </div>
      <label for="name-input">Name</label>
      <input id="name-input" type="text" maxlength="${MAX_NAME}" autocomplete="nickname" spellcheck="false" required>
      <div id="name-error" role="alert"></div>
      <div id="name-foot">
        <span>Remembered for this share in this browser; change it under the gear.</span>
        <button id="name-join" type="submit">Join</button>
      </div>
    </form>
  </dialog>
`;

export class Ui {
  readonly host: HTMLDivElement;
  readonly shadow: ShadowRoot;
  readonly cursors: HTMLDivElement;
  readonly highlight: HTMLDivElement;
  readonly editHighlight: HTMLDivElement;
  readonly edit: HTMLButtonElement;
  /** The four corner buttons in the control panel. */
  readonly corners: HTMLButtonElement[];
  readonly messageHighlight: HTMLDivElement;
  readonly bar: HTMLDivElement;
  readonly pill: HTMLButtonElement;
  readonly connection: HTMLSpanElement;
  readonly badge: HTMLSpanElement;
  readonly gear: HTMLButtonElement;
  readonly list: HTMLElement;
  readonly listHeader: HTMLElement;
  readonly listDot: HTMLSpanElement;
  readonly listCount: HTMLSpanElement;
  readonly connectionText: HTMLSpanElement;
  readonly collapse: HTMLButtonElement;
  readonly outboxList: HTMLOListElement;
  readonly panel: HTMLElement;
  readonly commentModeKey: HTMLButtonElement;
  readonly commentMode: HTMLButtonElement;
  readonly clearAnswersKey: HTMLButtonElement;
  readonly clearAnswers: HTMLButtonElement;
  readonly answerTimeout: HTMLInputElement;
  readonly showCursorKey: HTMLButtonElement;
  readonly showCursor: HTMLButtonElement;
  readonly nameField: HTMLInputElement;
  readonly panelMessage: HTMLDivElement;
  readonly hereCount: HTMLSpanElement;
  readonly avatars: HTMLUListElement;
  readonly toast: HTMLDivElement;
  readonly agentGeneral: HTMLDivElement;
  readonly agentAnchored: HTMLDivElement;
  readonly dialog: HTMLDialogElement;
  readonly dialogLabel: HTMLLabelElement;
  readonly form: HTMLFormElement;
  readonly selectionPreview: HTMLDivElement;
  readonly selectionText: HTMLQuoteElement;
  readonly textarea: HTMLTextAreaElement;
  readonly attachments: HTMLUListElement;
  readonly attachmentInput: HTMLInputElement;
  readonly attach: HTMLButtonElement;
  readonly missingAttachments: HTMLDivElement;
  readonly discardMissing: HTMLButtonElement;
  readonly status: HTMLDivElement;
  readonly reloadNotice: HTMLDivElement;
  readonly cancel: HTMLButtonElement;
  readonly send: HTMLButtonElement;
  readonly nameDialog: HTMLDialogElement;
  readonly nameForm: HTMLFormElement;
  readonly nameInput: HTMLInputElement;
  readonly nameError: HTMLDivElement;
  readonly nameJoin: HTMLButtonElement;
  private toastTimer: number | undefined;

  /** Adds the UI to the page; null when it is there already. */
  static install(): Ui | null {
    if (document.getElementById(HOST_ID)) return null;
    const host = document.createElement("div");
    host.id = HOST_ID;
    const shadow = host.attachShadow({ mode: "open" });
    shadow.innerHTML = MARKUP;
    document.body.append(host);
    return new Ui(host, shadow);
  }

  private constructor(host: HTMLDivElement, shadow: ShadowRoot) {
    const find = <T extends Element>(selector: string, type: new () => T): T => {
      const element = shadow.querySelector(selector);
      if (!(element instanceof type)) throw new Error(`jamparty: missing ${selector}`);
      return element;
    };
    this.host = host;
    this.shadow = shadow;
    this.cursors = find("#cursors", HTMLDivElement);
    this.highlight = find("#highlight", HTMLDivElement);
    this.editHighlight = find("#edit-highlight", HTMLDivElement);
    this.edit = find("#edit", HTMLButtonElement);
    this.corners = Array.from(shadow.querySelectorAll(".corner")).filter(
      (button) => button instanceof HTMLButtonElement,
    );
    this.messageHighlight = find("#message-highlight", HTMLDivElement);
    this.bar = find("#bar", HTMLDivElement);
    this.pill = find("#pill", HTMLButtonElement);
    this.connection = find("#connection", HTMLSpanElement);
    this.badge = find("#badge", HTMLSpanElement);
    this.gear = find("#gear", HTMLButtonElement);
    this.list = find("#list", HTMLElement);
    this.listHeader = find("#list-header", HTMLElement);
    this.listDot = find("#list-dot", HTMLSpanElement);
    this.listCount = find("#list-count", HTMLSpanElement);
    this.connectionText = find("#connection-text", HTMLSpanElement);
    this.collapse = find("#collapse", HTMLButtonElement);
    this.outboxList = find("#outbox-list", HTMLOListElement);
    this.panel = find("#panel", HTMLElement);
    this.commentModeKey = find("#comment-mode-key", HTMLButtonElement);
    this.commentMode = find("#comment-mode", HTMLButtonElement);
    this.clearAnswersKey = find("#clear-answers-key", HTMLButtonElement);
    this.clearAnswers = find("#clear-answers", HTMLButtonElement);
    this.answerTimeout = find("#answer-timeout", HTMLInputElement);
    this.showCursorKey = find("#show-cursor-key", HTMLButtonElement);
    this.showCursor = find("#show-cursor", HTMLButtonElement);
    this.nameField = find("#name-field", HTMLInputElement);
    this.panelMessage = find("#panel-message", HTMLDivElement);
    this.hereCount = find("#here-count", HTMLSpanElement);
    this.avatars = find("#avatars", HTMLUListElement);
    this.toast = find("#toast", HTMLDivElement);
    this.agentGeneral = find("#agent-general", HTMLDivElement);
    this.agentAnchored = find("#agent-anchored", HTMLDivElement);
    this.dialog = find("#jamparty-dialog", HTMLDialogElement);
    this.dialogLabel = find("#dialog-label", HTMLLabelElement);
    this.form = find("#jamparty-dialog form", HTMLFormElement);
    this.selectionPreview = find("#selection-preview", HTMLDivElement);
    this.selectionText = find("#selection-text", HTMLQuoteElement);
    this.textarea = find("textarea", HTMLTextAreaElement);
    this.attachments = find("#attachments", HTMLUListElement);
    this.attachmentInput = find("#attachment-input", HTMLInputElement);
    this.attach = find("#attach", HTMLButtonElement);
    this.missingAttachments = find("#missing-attachments", HTMLDivElement);
    this.discardMissing = find("#discard-missing", HTMLButtonElement);
    this.status = find("#status", HTMLDivElement);
    this.reloadNotice = find("#reload-notice", HTMLDivElement);
    this.cancel = find("#cancel", HTMLButtonElement);
    this.send = find("#send", HTMLButtonElement);
    this.nameDialog = find("#name-dialog", HTMLDialogElement);
    this.nameForm = find("#name-form", HTMLFormElement);
    this.nameInput = find("#name-input", HTMLInputElement);
    this.nameError = find("#name-error", HTMLDivElement);
    this.nameJoin = find("#name-join", HTMLButtonElement);
  }

  showToast(message: string): void {
    this.toast.textContent = message;
    this.toast.hidden = false;
    if (this.toastTimer !== undefined) clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => { this.toast.hidden = true; }, 2400);
  }

  setPending(pending: number, sending: number, refused: number): void {
    const parts = [`${pending} queued`];
    if (sending > 0) parts.push(`${sending} sending`);
    if (refused > 0) parts.push(`${refused} refused`);
    const text = parts.join(" · ");
    if (this.badge.textContent !== text) this.badge.textContent = text;
    this.badge.dataset.pending = String(pending);
    this.listCount.textContent = text;
  }
}
