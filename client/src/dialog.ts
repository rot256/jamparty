// The comment dialog: opened on an element (right-click), the whole page
// (`/`) or an answer; Enter queues the comment. While it is open, what is
// written is kept as a draft (see draft.ts) and comes back after a reload;
// once sent, it is the outbox's (sent again under its ref if need be).

import { type Captured, captureTarget, capturePageTarget } from "./capture";
import { AGENT_TARGET_PREFIX } from "./config";
import { type DraftKind, type DraftTarget, clearDraft, loadDraft, saveDraft } from "./draft";
import type { Outbox } from "./outbox";
import type { Ui } from "./ui";

const ELEMENT_LABEL = "Comment:";
const ANSWER_LABEL = "Comment on agent response:";
const GONE = "The element is gone; this will be a page comment.";

function labelFor(kind: DraftKind): string {
  switch (kind) {
    case "element":
    case "page":
      return ELEMENT_LABEL;
    case "answer":
      return ANSWER_LABEL;
  }
}

export class CommentDialog {
  private captured: Captured | null = null;
  /** What the dialog was opened on, which a draft keeps. */
  private target: DraftTarget = { kind: "page", selector: "" };
  /** The target was gone after a reload: a page comment now. */
  private unavailable = false;
  /** Handed to the outbox and on its way: no draft any more. */
  private sending = false;
  /** A live reload waiting for the dialog to close. */
  private pendingReload = false;

  constructor(
    private readonly ui: Ui,
    private readonly outbox: Outbox,
    /** How many of this joiner's comments the share holds. */
    private readonly pending: () => number,
  ) {
    const { dialog, form, textarea, cancel } = ui;
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.submit();
    });
    textarea.addEventListener("input", () => { this.keep(); });
    textarea.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        form.requestSubmit();
      }
    });
    cancel.addEventListener("click", () => { this.dismiss(); });
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      this.dismiss();
    });
    dialog.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const rect = dialog.getBoundingClientRect();
      if (
        event.clientX < rect.left || event.clientX > rect.right ||
        event.clientY < rect.top || event.clientY > rect.bottom
      ) this.dismiss();
    });
    dialog.addEventListener("close", () => {
      this.stopTracking();
      ui.highlight.style.display = "none";
    });
  }

  get open(): boolean {
    return this.ui.dialog.open;
  }

  /**
   * Closes the dialog at the person's request (Cancel, Escape, a click
   * outside), unless its comment is on its way: it stays until the joiner
   * answers, so no other comment starts meanwhile.
   */
  private dismiss(): void {
    if (!this.sending) this.close();
  }

  /** Reloads now, or when the dialog closes if it is open. */
  reloadWhenClosed(): void {
    if (this.ui.dialog.open) {
      this.pendingReload = true;
      this.ui.reloadNotice.hidden = false;
    } else {
      window.location.reload();
    }
  }

  /** Opens on `target`; `selector` names it when the page cannot (an answer). */
  openElement(target: Element, x: number, y: number, selector?: string): void {
    const kind = selector?.startsWith(AGENT_TARGET_PREFIX) ? "answer" : "element";
    const captured = captureTarget(target, selector);
    this.show(captured, x, y, { kind, selector: captured.selector });
  }

  openPage(): void {
    const x = window.innerWidth / 2;
    const y = window.innerHeight / 3;
    this.show(capturePageTarget(), x, y, { kind: "page", selector: "" });
  }

  /**
   * Opens the dialog again on the draft this page left, if any, its target
   * found by `find`. A target that is gone makes it a page comment, for
   * good: it stays one after further reloads, and the dialog says so.
   */
  restore(find: (selector: string) => Element | null): void {
    const draft = loadDraft();
    if (draft === null || this.ui.dialog.open) return;
    const { target, selectedText } = draft;
    let element: Element | null = null;
    if (target.kind !== "page" && !draft.unavailable) {
      try {
        element = find(target.selector);
      } catch {
        // Not a selector this page takes: gone, as far as it can tell.
      }
    }
    const page = window.location.href;
    let x = window.innerWidth / 2;
    let y = window.innerHeight / 3;
    let captured: Captured = { element: null, page, selector: "", selectedText };
    if (element !== null) {
      captured = { element, page, selector: target.selector, selectedText };
      const rect = element.getBoundingClientRect();
      x = rect.left;
      y = rect.bottom;
    }
    // Its title stays: a reply to an answer is one even if the answer is gone.
    this.show(captured, x, y, target);
    this.unavailable = target.kind !== "page" && element === null;
    this.ui.textarea.value = draft.comment;
    if (this.unavailable) this.ui.status.textContent = GONE;
    this.keep();
  }

  /** Keeps what the open dialog holds, for after a reload. */
  private keep(): void {
    const captured = this.captured;
    if (captured === null || !this.ui.dialog.open || this.sending) return;
    saveDraft({
      page: captured.page,
      target: this.target,
      unavailable: this.unavailable,
      selectedText: captured.selectedText,
      comment: this.ui.textarea.value,
    });
  }

  private readonly updateHighlight = (): void => {
    const { highlight, dialog } = this.ui;
    const element = this.captured?.element;
    if (!dialog.open || !element?.isConnected) {
      highlight.style.display = "none";
      return;
    }
    const rect = element.getBoundingClientRect();
    highlight.style.display = "block";
    highlight.style.left = `${rect.left}px`;
    highlight.style.top = `${rect.top}px`;
    highlight.style.width = `${rect.width}px`;
    highlight.style.height = `${rect.height}px`;
  };

  private stopTracking(): void {
    window.removeEventListener("resize", this.updateHighlight);
    window.removeEventListener("scroll", this.updateHighlight, true);
  }

  private place(x: number, y: number): void {
    const { dialog } = this.ui;
    const gap = 12;
    const width = dialog.offsetWidth;
    const height = dialog.offsetHeight;
    dialog.style.left = `${Math.max(gap, Math.min(x + gap, window.innerWidth - width - gap))}px`;
    dialog.style.top = `${Math.max(gap, Math.min(y + gap, window.innerHeight - height - gap))}px`;
  }

  private show(captured: Captured, x: number, y: number, target: DraftTarget): void {
    const ui = this.ui;
    if (ui.dialog.open) {
      ui.textarea.focus();
      return;
    }
    this.captured = captured;
    this.target = target;
    this.unavailable = false;
    ui.dialogLabel.textContent = labelFor(target.kind);
    ui.selectionPreview.hidden = captured.selectedText === null;
    ui.selectionText.textContent = captured.selectedText ?? "";
    ui.status.textContent = "";
    ui.reloadNotice.hidden = !this.pendingReload;
    ui.textarea.value = "";
    ui.dialog.showModal();
    this.place(x, y);
    this.updateHighlight();
    window.addEventListener("resize", this.updateHighlight);
    window.addEventListener("scroll", this.updateHighlight, true);
    ui.textarea.focus();
    this.keep();
  }

  close(): void {
    const ui = this.ui;
    // Firefox may leave focus in the closed dialog's textarea, where keys
    // (like `/`) would then count as typing.
    ui.textarea.blur();
    if (ui.dialog.open) ui.dialog.close();
    this.stopTracking();
    ui.highlight.style.display = "none";
    this.captured = null;
    this.unavailable = false;
    clearDraft();
    ui.selectionPreview.hidden = true;
    ui.selectionText.textContent = "";
    ui.status.textContent = "";
    ui.textarea.value = "";
    if (this.pendingReload) window.location.reload();
  }

  private submit(): void {
    const ui = this.ui;
    const captured = this.captured;
    // Enter submits even with the button disabled: one at a time.
    if (!captured || this.sending) return;
    const comment = ui.textarea.value.trim();
    if (!comment) {
      ui.status.textContent = "Enter a comment before queueing it.";
      ui.textarea.focus();
      return;
    }
    ui.send.disabled = true;
    ui.cancel.disabled = true;
    ui.status.textContent = "Queueing…";
    const draft = {
      page: captured.page,
      selector: captured.selector,
      selectedText: captured.selectedText,
      comment,
    };
    const ref = crypto.randomUUID();
    // The outbox has it (and its ref) before it goes: a reload before the
    // answer sends it again rather than bringing back a draft to send anew.
    this.sending = true;
    clearDraft();
    void this.outbox.submit(draft, ref).then((outcome) => {
      this.sending = false;
      ui.send.disabled = false;
      ui.cancel.disabled = false;
      if ("commented" in outcome) {
        this.close();
        ui.showToast(`Comment queued · ${this.pending()} pending`);
      } else if ("unsent" in outcome) {
        this.close();
        ui.showToast("Not sent: kept in the page, sent when the connection is back");
      } else {
        // Refused (too many, invalid): the dialog's again, and a draft.
        this.keep();
        ui.status.textContent = outcome.refused.error;
        ui.textarea.focus();
      }
    });
  }
}
