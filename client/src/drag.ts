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

export function dragBy(handle: HTMLElement, surface: HTMLElement, move: (point: Point) => void): void {
  let drag: { id: number; offset: Point; start: Point; moved: boolean } | undefined;
  handle.addEventListener("pointerdown", (event) => {
    if (!event.isPrimary || event.button !== 0 || drag !== undefined ||
        (event.target instanceof Element && event.target.closest("button, a"))) return;
    const rect = surface.getBoundingClientRect();
    drag = {
      id: event.pointerId,
      start: { x: event.clientX, y: event.clientY },
      offset: { x: event.clientX - rect.left, y: event.clientY - rect.top },
      moved: false,
    };
    event.preventDefault();
    handle.setPointerCapture(event.pointerId);
  });
  handle.addEventListener("pointermove", (event) => {
    if (drag?.id !== event.pointerId) return;
    if (!drag.moved && Math.hypot(event.clientX - drag.start.x, event.clientY - drag.start.y) < 4) return;
    drag.moved = true;
    handle.dataset.dragging = "";
    move(fitPosition({ x: event.clientX - drag.offset.x, y: event.clientY - drag.offset.y },
      surface.offsetWidth, surface.offsetHeight));
  });
  handle.addEventListener("click", (event) => {
    if (event.target === handle) event.stopPropagation();
  });
  handle.addEventListener("lostpointercapture", () => {
    drag = undefined;
    delete handle.dataset.dragging;
  });
}
