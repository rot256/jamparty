// Edit mode, like a design tool's select tool: while it is on, the element
// under the pointer is outlined and a left click comments on it instead of
// reaching the page (links do not navigate, buttons do not fire). It stays
// on after a comment; Escape (with no dialog open) or its button leaves it.
// It is only available while comment mode is on.
//
// Presses are taken on `window` in the capture phase, before any listener
// on the document or below it, so a site's delegated handlers do not run.
// (A site's own window capture listeners added before jampot loaded still
// see them; nothing in the page can be ahead of those.)

import type { CommentDialog } from "./dialog";
import type { Ui } from "./ui";

/** The mouse and pointer events a left click is made of. */
const PRESS_EVENTS = ["pointerdown", "mousedown", "pointerup", "mouseup", "dblclick"] as const;

export class EditMode {
  private on = false;
  private hovered: Element | null = null;
  /**
   * Called on each press edit mode takes from the page: the document's own
   * listeners (closing the panel and the list on outside presses) do not
   * see it.
   */
  onPagePress: () => void = () => undefined;

  constructor(
    private readonly ui: Ui,
    private readonly dialog: CommentDialog,
    /** Whether edit mode may be on: while comment mode is on. */
    private readonly allowed: () => boolean,
  ) {
    ui.edit.addEventListener("click", () => {
      if (this.on) this.leave();
      else this.enter();
    });
    for (const type of PRESS_EVENTS) {
      window.addEventListener(type, (event) => { this.swallow(event); }, true);
    }
    window.addEventListener("click", (event) => { this.select(event); }, true);
    document.addEventListener("pointermove", (event) => { this.hover(event); }, true);
    document.addEventListener("pointerleave", () => { this.outline(null); }, true);
    window.addEventListener("scroll", () => { this.outline(this.hovered); }, true);
    window.addEventListener("resize", () => { this.outline(this.hovered); });
    document.addEventListener("keydown", (event) => {
      if (!this.on || event.key !== "Escape" || this.dialog.open) return;
      event.preventDefault();
      this.leave();
    }, true);
  }

  /** Follows whether edit mode may be on; leaves it when it may not. */
  refresh(): void {
    const allowed = this.allowed();
    this.ui.edit.disabled = !allowed;
    if (!allowed && this.on) this.leave();
  }

  enter(): void {
    if (!this.allowed()) return;
    this.on = true;
    this.ui.edit.setAttribute("aria-pressed", "true");
  }

  leave(): void {
    this.on = false;
    this.ui.edit.setAttribute("aria-pressed", "false");
    this.outline(null);
  }

  /** The page element an event is about, or null for jampot's own overlay. */
  private pageTarget(event: Event): Element | null {
    if (event.composedPath().includes(this.ui.host)) return null;
    return event.target instanceof Element ? event.target : null;
  }

  private primary(event: Event): boolean {
    return !(event instanceof MouseEvent) || event.button === 0;
  }

  /** Keeps a left press from reaching the page: no focus, no selection. */
  private swallow(event: Event): void {
    if (!this.on || !this.primary(event) || this.pageTarget(event) === null) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.type === "pointerdown") this.onPagePress();
  }

  /** A left click on the page comments on what was clicked. */
  private select(event: MouseEvent): void {
    if (!this.on || event.button !== 0) return;
    const target = this.pageTarget(event);
    if (target === null) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    this.outline(null);
    this.dialog.openElement(target, event.clientX, event.clientY);
  }

  private hover(event: PointerEvent): void {
    if (!this.on || this.dialog.open) return;
    this.outline(this.pageTarget(event));
  }

  private outline(target: Element | null): void {
    this.hovered = target;
    const box = this.ui.editHighlight;
    if (!this.on || target === null || !target.isConnected) {
      box.style.display = "none";
      return;
    }
    const rect = target.getBoundingClientRect();
    box.style.display = "block";
    box.style.left = `${rect.left}px`;
    box.style.top = `${rect.top}px`;
    box.style.width = `${rect.width}px`;
    box.style.height = `${rect.height}px`;
  }
}
