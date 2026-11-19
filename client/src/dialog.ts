// The comment dialog: opened on an element (right-click), the whole page
// (`/`) or an answer; Enter queues the comment. While it is open, what is
// written is kept as a draft (see draft.ts) and comes back after a reload;
// once sent, it is the outbox's (sent again under its ref if need be).

import { upload } from "./attachments";
import { type Captured, captureTarget, capturePageTarget } from "./capture";
import { AGENT_TARGET_PREFIX } from "./config";
import { type DraftKind, type DraftTarget, clearDraft, loadDraft, saveDraft } from "./draft";
import { type Attachment, MAX_ATTACHMENTS, MAX_FILE_BYTES } from "./generated/api";
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
  private files: File[] = [];
  private missingFiles = 0;
  private position = { x: 0, y: 0 };
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
      void this.submit();
    });
    textarea.addEventListener("input", () => { this.keep(); });
    textarea.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        form.requestSubmit();
      }
    });
    ui.attach.addEventListener("click", () => { ui.attachmentInput.click(); });
    ui.attachmentInput.addEventListener("change", () => {
      this.addFiles(Array.from(ui.attachmentInput.files ?? []));
      ui.attachmentInput.value = "";
    });
    dialog.addEventListener("paste", (event) => {
      const files = Array.from(event.clipboardData?.files ?? []);
      if (files.length === 0) return;
      event.preventDefault();
      this.addFiles(files);
    });
    dialog.addEventListener("dragover", (event) => {
      if (event.dataTransfer?.types.includes("Files")) event.preventDefault();
    });
    dialog.addEventListener("drop", (event) => {
      const files = Array.from(event.dataTransfer?.files ?? []);
      if (files.length === 0) return;
      event.preventDefault();
      this.addFiles(files);
    });
    ui.discardMissing.addEventListener("click", () => {
      this.missingFiles = 0;
      this.showFiles();
      this.keep();
    });
    cancel.addEventListener("click", () => { this.dismiss(); });
    dialog.addEventListener("cancel", (event) => {
      // A file input's picker can dispatch a bubbling cancel event too.
      if (event.target !== dialog) return;
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

  private addFiles(files: File[]): void {
    if (!this.open || this.sending || files.length === 0) return;
    const { status } = this.ui;
    if (this.files.length + files.length > MAX_ATTACHMENTS) {
      status.textContent = `Attach at most ${MAX_ATTACHMENTS} files per comment.`;
      return;
    }
    const oversized = files.find((file) => file.size > MAX_FILE_BYTES);
    if (oversized) {
      status.textContent = `${oversized.name} exceeds 8 MiB.`;
      return;
    }
    this.files.push(...files);
    this.missingFiles = Math.max(0, this.missingFiles - files.length);
    status.textContent = this.unavailable ? GONE : "";
    this.showFiles();
    this.keep();
  }

  private showFiles(): void {
    const ui = this.ui;
    ui.textarea.required = this.files.length === 0;
    ui.attachments.hidden = this.files.length === 0;
    ui.missingAttachments.hidden = this.missingFiles === 0;
    ui.attachments.replaceChildren(...this.files.map((file, index) => {
      const item = document.createElement("li");
      const name = document.createElement("span");
      name.className = "attachment-name";
      name.textContent = file.name;
      name.title = file.name;
      const size = document.createElement("span");
      size.className = "attachment-size";
      size.textContent = file.size < 1024 ? `${file.size} B` : `${(file.size / 1024).toFixed(1)} KiB`;
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "×";
      remove.setAttribute("aria-label", `Remove ${file.name}`);
      remove.disabled = this.sending;
      remove.addEventListener("click", () => {
        this.files.splice(index, 1);
        this.showFiles();
        this.keep();
        ui.attach.focus();
      });
      item.append(name, size, remove);
      return item;
    }));
    if (this.open) this.place();
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
    this.missingFiles = draft.fileCount;
    this.showFiles();
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
      fileCount: this.files.length + this.missingFiles,
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

  private place(): void {
    const { dialog } = this.ui;
    const { x, y } = this.position;
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
    this.position = { x, y };
    ui.dialog.showModal();
    this.place();
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
    this.files = [];
    this.missingFiles = 0;
    this.showFiles();
    ui.selectionPreview.hidden = true;
    ui.selectionText.textContent = "";
    ui.status.textContent = "";
    ui.textarea.value = "";
    if (this.pendingReload) window.location.reload();
  }

  private setSending(sending: boolean): void {
    this.sending = sending;
    const { send, cancel, attach, discardMissing, textarea, attachments } = this.ui;
    send.disabled = sending;
    cancel.disabled = sending;
    attach.disabled = sending;
    discardMissing.disabled = sending;
    textarea.readOnly = sending;
    for (const button of attachments.querySelectorAll("button")) button.disabled = sending;
  }

  private async submit(): Promise<void> {
    const ui = this.ui;
    const captured = this.captured;
    // Enter submits even with the button disabled: one at a time.
    if (!captured || this.sending) return;
    if (this.missingFiles !== 0) {
      ui.status.textContent = "Attach the missing files again or choose to continue without them.";
      return;
    }
    const comment = ui.textarea.value.trim();
    if (!comment && this.files.length === 0) {
      ui.status.textContent = "Enter a comment before queueing it.";
      ui.textarea.focus();
      return;
    }
    this.setSending(true);
    const attachments: Attachment[] = [];
    try {
      for (const file of this.files) {
        ui.status.textContent = `Uploading ${file.name}…`;
        attachments.push(await upload(file));
      }
    } catch (error) {
      this.setSending(false);
      ui.status.textContent = error instanceof Error ? error.message : "Could not upload the attachment.";
      return;
    }
    ui.status.textContent = "Queueing…";
    const draft = {
      page: captured.page,
      selector: captured.selector,
      selectedText: captured.selectedText,
      comment,
      ...(attachments.length > 0 ? { attachments } : {}),
    };
    const ref = crypto.randomUUID();
    // The outbox has the uploaded attachments and ref before the socket
    // sends the comment, so it can retry across reconnects and reloads.
    clearDraft();
    const outcome = await this.outbox.submit(draft, ref);
    this.setSending(false);
    if ("commented" in outcome) {
      this.close();
      ui.showToast(`Comment queued · ${this.pending()} pending`);
    } else if ("unsent" in outcome) {
      this.close();
      ui.showToast("Not sent: kept in the page, sent when the connection is back");
    } else {
      this.keep();
      ui.status.textContent = outcome.refused.error;
      ui.textarea.focus();
    }
  }
}
