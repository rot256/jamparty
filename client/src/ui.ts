// The overlay's elements, in a shadow root of their own: the pill with
// the connection dot and the gear (bottom right), the comment list the
// pill expands into, the control panel, other people's cursors, the
// agent's answers and the dialogs.

import { ANSWER_TIMEOUT_PRESETS, answerTimeoutLabel } from "./answer-timeout";
import { HOST_ID } from "./config";
import { DEFAULT_ANSWER_TIMEOUT, DEFAULT_CLEAR_ANSWERS_KEY, MAX_COMMENT, MAX_NAME } from "./generated/api";
import type { TextSize } from "./generated/api";
import { styles } from "./styles";

const TEXT_SCALES: Readonly<Record<TextSize, number>> = {
  small: 1, default: 1.125, large: 1.25, larger: 1.375,
};

const GEAR_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"></path></svg>`;

const PEN_ICON = `<svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 3 5 5M3 21l5-1L21 7a2.1 2.1 0 0 0-5-5L3 15z"></path></svg>`;

const PAPERCLIP_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21 11-9 9a5 5 0 0 1-7-7l10-10a3.5 3.5 0 0 1 5 5L10 18a1.5 1.5 0 0 1-2-2l9-9"></path></svg>`;

const MARKUP = `
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
  <section id="list" aria-label="Comments and replies" hidden>
    <header id="list-header">
      <div id="list-summary">
        <span id="list-dot" class="dot" data-state="amber"></span>
        <span id="list-count">0 queued</span>
        <span id="connection-text" role="status">· connecting</span>
      </div>
      <button id="collapse" type="button" aria-label="Collapse">▾</button>
    </header>
    <div id="list-content">
      <ol id="outbox-list"></ol>
      <ul id="answer-list" aria-label="Replies"></ul>
    </div>
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
    <div class="control" id="hide-answers-row">
      <span class="label" id="hide-answers-label">Hide answers</span>
      <span class="segments" role="group" aria-labelledby="hide-answers-label">
        ${ANSWER_TIMEOUT_PRESETS.map((seconds) => `
          <button class="segment" type="button" data-answer-timeout="${seconds}" aria-pressed="${seconds === DEFAULT_ANSWER_TIMEOUT}">${answerTimeoutLabel(seconds)}</button>
        `).join("")}
        <button class="segment" id="custom-answer-timeout" type="button" aria-pressed="true" hidden></button>
      </span>
    </div>
    <div class="control" id="corner-row">
      <span class="label" id="corner-label">Corner</span>
      <span id="corners" class="segments" role="group" aria-labelledby="corner-label">
        <button class="corner segment" type="button" data-corner="top-left" aria-label="Top left" aria-pressed="false">↖</button>
        <button class="corner segment" type="button" data-corner="top-right" aria-label="Top right" aria-pressed="false">↗</button>
        <button class="corner segment" type="button" data-corner="bottom-left" aria-label="Bottom left" aria-pressed="false">↙</button>
        <button class="corner segment" type="button" data-corner="bottom-right" aria-label="Bottom right" aria-pressed="true">↘</button>
      </span>
    </div>
    <div class="control">
      <span class="label" id="theme-label">Theme</span>
      <span class="segments" role="group" aria-labelledby="theme-label">
        <button class="segment" type="button" data-theme="auto" aria-pressed="true">Auto</button>
        <button class="segment" type="button" data-theme="dark" aria-pressed="false">Dark</button>
        <button class="segment" type="button" data-theme="light" aria-pressed="false">Light</button>
      </span>
    </div>
    <div class="control" id="text-size-row">
      <span class="label" id="text-size-label">Text size</span>
      <span class="segments" role="group" aria-labelledby="text-size-label">
        <button class="segment" type="button" data-text-size="small" aria-pressed="false">Small</button>
        <button class="segment" type="button" data-text-size="default" aria-pressed="true">Default</button>
        <button class="segment" type="button" data-text-size="large" aria-pressed="false">Large</button>
        <button class="segment" type="button" data-text-size="larger" aria-pressed="false">Larger</button>
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
      <header id="dialog-header">
        <label id="dialog-label" for="comment">Comment:</label>
      </header>
      <div id="selection-preview" hidden>
        <div id="selection-label">Selected text</div>
        <blockquote id="selection-text"></blockquote>
      </div>
      <textarea id="comment" name="comment" autofocus maxlength="${MAX_COMMENT}" required></textarea>
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
  readonly answerList: HTMLUListElement;
  readonly panel: HTMLElement;
  readonly commentModeKey: HTMLButtonElement;
  readonly commentMode: HTMLButtonElement;
  readonly clearAnswersKey: HTMLButtonElement;
  readonly customAnswerTimeout: HTMLButtonElement;
  readonly clearAnswers: HTMLButtonElement;
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
  readonly dialogHeader: HTMLElement;
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
  private readonly root: HTMLDivElement;
  private scale = TEXT_SCALES.default;

  get textScale(): number {
    return this.scale;
  }

  setTextSize(size: TextSize): void {
    this.scale = TEXT_SCALES[size];
    this.root.style.setProperty("--jp-unit", `${this.scale}px`);
  }

  private toastTimer: number | undefined;

  /** Adds the UI to the page; null when it is there already. */
  static install(): Ui | null {
    if (document.getElementById(HOST_ID)) return null;
    const host = document.createElement("div");
    host.id = HOST_ID;
    const shadow = host.attachShadow({ mode: "open" });
    shadow.innerHTML = `<style>${styles}</style><div id="overlay-root">${MARKUP}</div>`;
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
    this.root = find("#overlay-root", HTMLDivElement);
    this.setTextSize("default");
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
    this.answerList = find("#answer-list", HTMLUListElement);
    this.panel = find("#panel", HTMLElement);
    this.commentModeKey = find("#comment-mode-key", HTMLButtonElement);
    this.commentMode = find("#comment-mode", HTMLButtonElement);
    this.clearAnswersKey = find("#clear-answers-key", HTMLButtonElement);
    this.customAnswerTimeout = find("#custom-answer-timeout", HTMLButtonElement);
    this.clearAnswers = find("#clear-answers", HTMLButtonElement);
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
    this.dialogHeader = find("#dialog-header", HTMLElement);
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
