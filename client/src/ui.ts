// The comment UI's elements, in a shadow root of their own.

import { HOST_ID } from "./config";
import { styles } from "./styles";

const MARKUP = `
  <style>${styles}</style>
  <div id="highlight" aria-hidden="true"></div>
  <div id="message-highlight" aria-hidden="true"></div>
  <div id="bar">
    <div id="connection" data-state="amber" role="img" aria-label="Connecting" title="Connecting"></div>
    <div id="badge" role="status" aria-live="polite">0 queued</div>
  </div>
  <div id="outbox" hidden>
    <div id="connection-text" role="status" hidden></div>
    <ul id="outbox-list"></ul>
  </div>
  <div id="toast" role="status" aria-live="polite" hidden></div>
  <div id="agent-log" role="log" aria-live="polite" aria-relevant="additions">
    <div id="agent-general"></div>
    <div id="agent-anchored"></div>
  </div>
  <dialog id="jampot-dialog" aria-labelledby="dialog-label">
    <form>
      <label id="dialog-label" for="comment">Comment:</label>
      <div id="selection-preview" hidden>
        <div id="selection-label">Selected text</div>
        <blockquote id="selection-text"></blockquote>
      </div>
      <textarea id="comment" name="comment" maxlength="10000" required></textarea>
      <div id="status" role="alert"></div>
      <div id="reload-notice" role="status" hidden>
        Files changed. Reloading when this edit closes.
      </div>
      <div class="actions">
        <button id="cancel" type="button">Cancel</button>
        <button id="send" type="submit">Queue comment</button>
      </div>
    </form>
  </dialog>
`;

export class Ui {
  readonly host: HTMLDivElement;
  readonly shadow: ShadowRoot;
  readonly highlight: HTMLDivElement;
  readonly messageHighlight: HTMLDivElement;
  readonly badge: HTMLDivElement;
  readonly connection: HTMLDivElement;
  readonly outbox: HTMLDivElement;
  readonly connectionText: HTMLDivElement;
  readonly outboxList: HTMLUListElement;
  readonly toast: HTMLDivElement;
  readonly agentGeneral: HTMLDivElement;
  readonly agentAnchored: HTMLDivElement;
  readonly dialog: HTMLDialogElement;
  readonly dialogLabel: HTMLLabelElement;
  readonly form: HTMLFormElement;
  readonly selectionPreview: HTMLDivElement;
  readonly selectionText: HTMLQuoteElement;
  readonly textarea: HTMLTextAreaElement;
  readonly status: HTMLDivElement;
  readonly reloadNotice: HTMLDivElement;
  readonly cancel: HTMLButtonElement;
  readonly send: HTMLButtonElement;
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
      if (!(element instanceof type)) throw new Error(`jampot: missing ${selector}`);
      return element;
    };
    this.host = host;
    this.shadow = shadow;
    this.highlight = find("#highlight", HTMLDivElement);
    this.messageHighlight = find("#message-highlight", HTMLDivElement);
    this.badge = find("#badge", HTMLDivElement);
    this.connection = find("#connection", HTMLDivElement);
    this.outbox = find("#outbox", HTMLDivElement);
    this.connectionText = find("#connection-text", HTMLDivElement);
    this.outboxList = find("#outbox-list", HTMLUListElement);
    this.toast = find("#toast", HTMLDivElement);
    this.agentGeneral = find("#agent-general", HTMLDivElement);
    this.agentAnchored = find("#agent-anchored", HTMLDivElement);
    this.dialog = find("dialog", HTMLDialogElement);
    this.dialogLabel = find("#dialog-label", HTMLLabelElement);
    this.form = find("form", HTMLFormElement);
    this.selectionPreview = find("#selection-preview", HTMLDivElement);
    this.selectionText = find("#selection-text", HTMLQuoteElement);
    this.textarea = find("textarea", HTMLTextAreaElement);
    this.status = find("#status", HTMLDivElement);
    this.reloadNotice = find("#reload-notice", HTMLDivElement);
    this.cancel = find("#cancel", HTMLButtonElement);
    this.send = find("#send", HTMLButtonElement);
  }

  showToast(message: string): void {
    this.toast.textContent = message;
    this.toast.hidden = false;
    if (this.toastTimer !== undefined) clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => { this.toast.hidden = true; }, 2400);
  }

  setPending(pending: number): void {
    this.badge.textContent = `${pending} queued`;
    this.badge.dataset.pending = String(pending);
  }
}
