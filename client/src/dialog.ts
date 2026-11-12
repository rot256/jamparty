// The comment dialog: opened on an element (right-click), the whole page
// (`/`) or an answer; Enter queues the comment.

import { SubmitError, submit } from "./api";
import { type Captured, captureTarget, capturePageTarget } from "./capture";
import type { Outbox } from "./outbox";
import type { Ui } from "./ui";

export const ELEMENT_LABEL = "Comment:";
export const ANSWER_LABEL = "Comment on agent response:";

export class CommentDialog {
  private captured: Captured | null = null;
  /** A live reload waiting for the dialog to close. */
  private pendingReload = false;

  constructor(
    private readonly ui: Ui,
    private readonly outbox: Outbox,
  ) {
    const { dialog, form, textarea, cancel } = ui;
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.submit();
    });
    textarea.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        form.requestSubmit();
      }
    });
    cancel.addEventListener("click", () => { this.close(); });
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      this.close();
    });
    dialog.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const rect = dialog.getBoundingClientRect();
      if (
        event.clientX < rect.left || event.clientX > rect.right ||
        event.clientY < rect.top || event.clientY > rect.bottom
      ) this.close();
    });
    dialog.addEventListener("close", () => {
      this.stopTracking();
      ui.highlight.style.display = "none";
    });
  }

  get open(): boolean {
    return this.ui.dialog.open;
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

  openElement(target: Element, x: number, y: number, selector?: string, label = ELEMENT_LABEL): void {
    this.show(captureTarget(target, selector), x, y, label);
  }

  openPage(): void {
    this.show(capturePageTarget(), window.innerWidth / 2, window.innerHeight / 3, ELEMENT_LABEL);
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

  private show(captured: Captured, x: number, y: number, label: string): void {
    const ui = this.ui;
    if (ui.dialog.open) {
      ui.textarea.focus();
      return;
    }
    this.captured = captured;
    ui.dialogLabel.textContent = label;
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
    ui.selectionPreview.hidden = true;
    ui.selectionText.textContent = "";
    ui.status.textContent = "";
    ui.textarea.value = "";
    if (this.pendingReload) window.location.reload();
  }

  private submit(): void {
    const ui = this.ui;
    const captured = this.captured;
    if (!captured) return;
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
    const at = Date.now();
    submit({ ref, ...draft })
      .then((queued) => {
        this.outbox.add({ draft, ref, id: queued.id, state: "queued", at });
        this.outbox.onPending(queued.pending);
        this.close();
        ui.showToast(`Comment queued · ${queued.pending} pending`);
      })
      .catch((error: unknown) => {
        if (error instanceof SubmitError && error.unsent) {
          this.outbox.add({ draft, ref, id: null, state: "unsent", at });
          this.close();
          ui.showToast("Not sent: kept in the page, sent when the connection is back");
          return;
        }
        ui.status.textContent = error instanceof Error ? error.message : "Could not queue comment";
        ui.textarea.focus();
      })
      .finally(() => {
        ui.send.disabled = false;
        ui.cancel.disabled = false;
      });
  }
}
