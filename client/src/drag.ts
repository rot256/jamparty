import { answerViewport, clamp } from "./answer-placement";

export interface Point {
  x: number;
  y: number;
}

export function fitPosition({ x, y }: Point, width: number, height: number): Point {
  const view = answerViewport();
  return {
    x: clamp(x, view.left, view.right - width),
    y: clamp(y, view.top, view.bottom - height),
  };
}

const directions: Readonly<Record<string, Point>> = {
  ArrowLeft: { x: -1, y: 0 },
  ArrowRight: { x: 1, y: 0 },
  ArrowUp: { x: 0, y: -1 },
  ArrowDown: { x: 0, y: 1 },
};

/** Adds pointer dragging and a keyboard move control. Returns a cleanup for an active move. */
export function moveBy(
  handle: HTMLElement,
  surface: HTMLElement,
  move: (point: Point, width: number, applied?: (point: Point) => void) => void,
  savePlacement: () => () => void,
  anchorOrigin?: () => Point | undefined,
  fit: (point: Point, width: number, height: number, origin?: Point) => Point = fitPosition,
): () => void {
  const button = document.createElement("button");
  button.className = "move-control";
  button.type = "button";
  button.setAttribute("aria-label", "Move");
  button.title = "Move with arrow keys";
  button.innerHTML = '<svg aria-hidden="true" width="12" height="14" viewBox="0 0 12 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 3h0m4 0h0M4 7h0m4 0h0m-4 4h0m4 0h0"/></svg>';
  handle.prepend(button);

  let drag: { id: number; start: Point; rect: DOMRect; moved: boolean } | undefined;
  let keyboard: { point: Point; rect: DOMRect } | undefined;
  let restore: (() => void) | undefined;

  const end = (): void => {
    const id = drag?.id;
    drag = undefined;
    keyboard = undefined;
    restore = undefined;
    delete handle.dataset.dragging;
    window.removeEventListener("keydown", escape, true);
    if (id !== undefined && handle.hasPointerCapture(id)) handle.releasePointerCapture(id);
  };
  function escape(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopImmediatePropagation();
    restore?.();
    end();
  }
  const begin = (): void => {
    restore = savePlacement();
    window.addEventListener("keydown", escape, true);
  };

  handle.addEventListener("pointerdown", (event) => {
    const control = event.target instanceof Element ? event.target.closest("button, a") : null;
    if (!event.isPrimary || event.button !== 0 || drag !== undefined ||
        (control !== null && control !== button)) return;
    if (control === button) button.focus({ preventScroll: true });
    end();
    drag = {
      id: event.pointerId,
      start: { x: event.clientX, y: event.clientY },
      rect: surface.getBoundingClientRect(),
      moved: false,
    };
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
    begin();
  });
  handle.addEventListener("pointermove", (event) => {
    if (drag?.id !== event.pointerId) return;
    const x = event.clientX - drag.start.x;
    const y = event.clientY - drag.start.y;
    if (!drag.moved && Math.hypot(x, y) < 4) return;
    drag.moved = true;
    handle.dataset.dragging = "";
    move({ x: drag.rect.left + x, y: drag.rect.top + y }, drag.rect.width);
  });
  handle.addEventListener("click", (event) => {
    if (event.target === handle) event.stopPropagation();
  });
  handle.addEventListener("lostpointercapture", (event) => {
    if (drag?.id === event.pointerId) end();
  });

  const moveWithKeys = (event: KeyboardEvent): void => {
    if (drag !== undefined || event.isComposing || event.altKey || event.ctrlKey || event.metaKey ||
        !event.composedPath().includes(button)) return;
    const direction = Object.hasOwn(directions, event.key) ? directions[event.key] : undefined;
    if (direction === undefined && (event.key !== "Enter" || keyboard === undefined)) return;
    event.preventDefault();
    event.stopPropagation();
    if (direction === undefined) {
      end();
      return;
    }
    const origin = anchorOrigin?.();
    if (keyboard === undefined) {
      const rect = surface.getBoundingClientRect();
      keyboard = { point: { x: rect.left - (origin?.x ?? 0), y: rect.top - (origin?.y ?? 0) }, rect };
      begin();
    }
    const { point, rect } = keyboard;
    const step = event.shiftKey ? 64 : 16;
    const next = { x: point.x + step * direction.x, y: point.y + step * direction.y };
    const x = origin?.x ?? 0;
    const y = origin?.y ?? 0;
    const applied = fit({ x: next.x + x, y: next.y + y }, rect.width, rect.height, origin);
    keyboard.point = { x: applied.x - x, y: applied.y - y };
    const active = keyboard;
    move(applied, rect.width, (point) => {
      if (keyboard === active) keyboard.point = point;
    });
  };
  const stopListening = (): void => {
    window.removeEventListener("keydown", moveWithKeys, true);
    if (keyboard !== undefined) end();
  };
  // Focused move controls take arrows before configurable page-wide shortcuts.
  button.addEventListener("focus", () => { window.addEventListener("keydown", moveWithKeys, true); });
  button.addEventListener("blur", stopListening);
  return () => {
    stopListening();
    end();
  };
}
