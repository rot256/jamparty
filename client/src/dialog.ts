// The comment dialog: opened on an element (right-click), the whole page
// (`/`) or an answer; Enter saves and starts sending the comment. While it is open, what is
// written is kept as a draft (see draft.ts) and comes back after a reload;
// once sent, it is the outbox's (sent again under its ref if need be).

import { type Captured, captureTarget, capturePageTarget } from "./capture";
import { AGENT_TARGET_PREFIX } from "./config";
import { type DraftKind, type DraftTarget, type SavedDraft, clearDraft, loadDraft, saveDraft } from "./draft";
import { type Attachment, MAX_ATTACHMENTS, MAX_FILE_BYTES } from "./generated/api";
import type { Outbox, OutboxEntry } from "./outbox";
import type { Ui } from "./ui";

const ELEMENT_LABEL = "Comment:";
const ANSWER_LABEL = "Comment on agent response:";
const GONE = "The element is gone; this will be a page comment.";

function labelFor(kind: DraftKind): string {
  switch (kind) {
    case "element":
    case "page":
    case "pdf":
      return ELEMENT_LABEL;
    case "answer":
      return ANSWER_LABEL;
  }
}

export class CommentDialog {
  onOpen: (captured: Captured) => void = () => undefined;
  onClose: () => void = () => undefined;
  private files: (File | Attachment)[] = [];
  private editing: OutboxEntry | undefined;
  private originals: (File | Attachment)[] = [];
  private missingFiles = 0;
  private position = { x: 0, y: 0 };
  private captured: Captured | null = null;
  /** What the dialog was opened on, which a draft keeps. */
  private target: DraftTarget = { kind: "page", selector: "" };
  /** The target was gone after a reload: a page comment now. */
  private unavailable = false;
  /** Saving this draft in the outbox. */
  private sending = false;
  /** A live reload waiting for the dialog to close. */
  private pendingReload = false;

  constructor(
    private readonly ui: Ui,
    private readonly outbox: Outbox,
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
      if (file instanceof File) {
        size.textContent = file.size < 1024 ? `${file.size} B` : `${(file.size / 1024).toFixed(1)} KiB`;
      }
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
   * outside), unless its comment is being saved locally.
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

  openPdf(captured: Captured, x: number, y: number): void {
    this.show(captured, x, y, { kind: "pdf", selector: captured.selector });
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
  async restore(find: (selector: string) => Element | null): Promise<void> {
    const draft = loadDraft();
    if (draft === null || this.open) return;
    if (draft.editRef === undefined) {
      this.showSaved(draft, find);
      return;
    }
    const entry = this.outbox.entries.find((candidate) => candidate.ref === draft.editRef);
    if (entry?.state !== "failed") {
      clearDraft();
      return;
    }
    await this.edit(entry, find, draft);
  }

  async edit(
    entry: OutboxEntry,
    find: (selector: string) => Element | null,
    savedDraft?: SavedDraft,
  ): Promise<boolean> {
    const { draft } = entry;
    const saved = entry.files === undefined ? { files: [] } : await this.outbox.savedFiles(entry).catch(
      () => ({ files: [] }),
    );
    if (this.open || !this.outbox.entries.includes(entry) || entry.state !== "failed") return false;
    const selector = draft.selector;
    const kind = draft.pdf !== undefined ? "pdf" : selector.startsWith(AGENT_TARGET_PREFIX) ? "answer" :
      selector === "" ? "page" : "element";
    const refs = draft.attachments ?? [];
    // Prefer the original bytes so a correction can recover an evicted resource.
    const existing = entry.uploaded && saved.files.length > 0 ? refs.slice(0, -saved.files.length) : refs;
    const originals = [...existing, ...saved.files];
    const files = savedDraft === undefined ? originals : (savedDraft.originalFiles ?? []).flatMap(
      (index) => originals[index] === undefined ? [] : [originals[index]],
    );
    const fileCount = savedDraft?.fileCount ?? existing.length +
      (entry.uploaded === true ? saved.files.length : entry.files?.length ?? 0);
    this.editing = entry;
    this.originals = originals;
    this.showSaved({
      ...draft,
      page: window.location.href,
      target: { kind, selector },
      unavailable: false,
      ...savedDraft,
      fileCount: Math.max(0, fileCount - files.length),
    }, find, files);
    return true;
  }

  private showSaved(
    draft: SavedDraft,
    find: (selector: string) => Element | null,
    files: (File | Attachment)[] = [],
  ): void {
    const { target, selectedText } = draft;
    let element: Element | null = null;
    if (target.kind !== "page" && target.kind !== "pdf" && !draft.unavailable) {
      try {
        element = find(target.selector);
      } catch {
        // Not a selector this page takes: gone, as far as it can tell.
      }
    }
    const page = draft.page;
    let x = window.innerWidth / 2;
    let y = window.innerHeight / 3;
    let captured: Captured = { element: null, page, selector: "", selectedText };
    if (element !== null) {
      captured = { element, page, selector: target.selector, selectedText };
      const rect = element.getBoundingClientRect();
      x = rect.left;
      y = rect.bottom;
    }
    if (target.kind === "pdf" && draft.pdf !== undefined) {
      captured = { ...captured, selector: target.selector, pdf: draft.pdf };
    }
    // Its title stays: a reply to an answer is one even if the answer is gone.
    this.show(captured, x, y, target);
    this.unavailable = target.kind !== "page" && target.kind !== "pdf" && element === null;
    this.ui.textarea.value = draft.comment;
    this.files = files;
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
      ...(captured.pdf === undefined ? {} : { pdf: captured.pdf }),
      comment: this.ui.textarea.value,
      fileCount: this.files.length + this.missingFiles,
      ...(this.editing === undefined ? {} : {
        editRef: this.editing.ref,
        originalFiles: this.files.map((file) => this.originals.indexOf(file)).filter((index) => index !== -1),
      }),
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
    // The quote is already captured. Clear the page range before focus
    // moves to the textarea, and again after the browser restores focus.
    window.getSelection()?.removeAllRanges();
    ui.dialog.showModal();
    this.onOpen(captured);
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
    window.getSelection()?.removeAllRanges();
    this.onClose();
    this.stopTracking();
    ui.highlight.style.display = "none";
    this.captured = null;
    this.editing = undefined;
    this.originals = [];
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
    const attachments = this.files.filter((file): file is Attachment => !(file instanceof File));
    const draft = {
      page: captured.page,
      selector: captured.selector,
      selectedText: captured.selectedText,
      ...(captured.pdf === undefined ? {} : { pdf: captured.pdf }),
      comment,
      ...(attachments.length > 0 ? { attachments } : {}),
    };
    try {
      await this.outbox.enqueue(draft, this.files.filter((file) => file instanceof File), this.editing);
      this.close();
    } catch (error) {
      ui.status.textContent = error instanceof Error ? error.message : "Could not save the comment.";
    } finally {
      this.setSending(false);
    }
  }
}
