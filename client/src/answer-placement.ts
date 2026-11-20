// Keep answer cards outside their targets, even when the viewport is narrow.
export interface Box {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

type Side = "right" | "left" | "below" | "above";

interface Band {
  side: Side;
  area: Box;
}

export interface Placement {
  box: Box;
  side: Side;
  maxHeight: number;
}

const EDGE = 8;
const GAP = 12;
const PAGE_GAP = 24;
const MIN_WIDTH = 200;
const MAX_WIDTH = 340;
const MIN_HEIGHT = 48;

export function answerViewport(): Box {
  const viewport = window.visualViewport;
  const x = viewport?.offsetLeft ?? 0;
  const y = viewport?.offsetTop ?? 0;
  return {
    left: x + EDGE,
    top: y + EDGE,
    right: x + (viewport?.width ?? window.innerWidth) - EDGE,
    bottom: y + (viewport?.height ?? window.innerHeight) - EDGE,
  };
}

function overlaps(a: Box, b: Box): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

function clamp(value: number, lower: number, upper: number): number {
  return Math.max(lower, Math.min(value, upper));
}

function sideBands(view: Box, target: Box, gap: number): Band[] {
  const right = { ...view, left: Math.max(view.left, target.right + gap) };
  const left = { ...view, right: Math.min(view.right, target.left - gap) };
  const bands: Band[] = [{ side: "right", area: right }, { side: "left", area: left }];
  return bands.sort((a, b) => (b.area.right - b.area.left) - (a.area.right - a.area.left));
}

export function placeAnswer(
  heightAt: (width: number) => number, view: Box, target: Box, page: Box | undefined, occupied: readonly Box[],
): Placement | undefined {
  const bands: Band[] = [
    ...(page === undefined ? [] : sideBands(view, page, PAGE_GAP)),
    ...sideBands(view, target, GAP),
    { side: "below", area: { ...view, top: Math.max(view.top, target.bottom + GAP) } },
    { side: "above", area: { ...view, bottom: Math.min(view.bottom, target.top - GAP) } },
  ];
  for (const allowClipping of [false, true]) {
    for (const { side, area } of bands) {
      const width = Math.min(MAX_WIDTH, Math.floor(area.right - area.left));
      if (width < Math.min(MIN_WIDTH, Math.floor(view.right - view.left)) || width <= 0) continue;
      const height = heightAt(width);
      const horizontal = side === "right" || side === "left";
      const desiredLeft = clamp(target.left, area.left, area.right - width);
      const lefts = horizontal
        ? [side === "right" ? area.left : area.right - width]
        : [desiredLeft, ...occupied.flatMap((box) => [box.right + GAP, box.left - width - GAP])]
          .filter((left) => left >= area.left && left + width <= area.right);
      let best: { placement: Placement; score: number } | undefined;
      for (const left of lefts) {
        const obstacles = occupied.filter((box) => left < box.right + GAP && left + width > box.left - GAP);
        const desiredTop = side === "above" ? target.top - GAP - height : target.top;
        const tops = [
          clamp(desiredTop, area.top, area.bottom - height),
          ...obstacles.flatMap((box) => [box.bottom + GAP, box.top - GAP - height]),
          area.top,
        ];
        for (const top of tops) {
          if (top < area.top || top >= area.bottom) continue;
          // Bands assume the target lies inside its page; check every
          // candidate against the target itself as well.
          if (overlaps({ left, right: left + width, top, bottom: top + Math.min(height, area.bottom - top) }, target)) {
            continue;
          }
          let bottom = area.bottom;
          for (const box of obstacles) {
            if (top >= box.bottom + GAP) continue;
            bottom = Math.min(bottom, box.top - GAP);
          }
          const maxHeight = bottom - top;
          if (maxHeight < height && (!allowClipping || maxHeight < MIN_HEIGHT)) continue;
          const fittedHeight = Math.min(maxHeight, height);
          const score = Math.abs(top - desiredTop) + Math.abs(left - desiredLeft) +
            (height - fittedHeight) * 2;
          if (best === undefined || score < best.score) {
            best = {
              placement: { side, maxHeight, box: { left, right: left + width, top, bottom: top + fittedHeight } },
              score,
            };
          }
        }
      }
      if (best !== undefined) return best.placement;
    }
  }
  return undefined;
}

function drawBox(node: HTMLElement, box: Box): void {
  node.style.left = `${box.left}px`;
  node.style.top = `${box.top}px`;
  node.style.width = `${box.right - box.left}px`;
  node.style.height = `${box.bottom - box.top}px`;
}

export function drawAnswerLink(
  marker: HTMLElement, line: HTMLElement, view: Box, target: Box, { side, box }: Placement,
): void {
  const horizontal = side === "right" || side === "left";
  marker.hidden = target.bottom <= view.top || target.top >= view.bottom ||
    target.right <= view.left || target.left >= view.right;
  line.hidden = marker.hidden;
  if (marker.hidden) return;

  const edge = {
    right: { x: target.right + 3, y: box.top + 16 },
    left: { x: target.left - 3, y: box.top + 16 },
    below: { x: box.left + 16, y: target.bottom + 3 },
    above: { x: box.left + 16, y: target.top - 3 },
  }[side];
  const x = clamp(edge.x, Math.max(view.left, target.left), Math.min(view.right, target.right));
  const y = clamp(edge.y, Math.max(view.top, target.top), Math.min(view.bottom, target.bottom));
  const fromX = horizontal ? edge.x : x;
  const fromY = horizontal ? y : edge.y;
  const toX = horizontal ? (side === "right" ? box.left : box.right) : clamp(x, box.left + 8, box.right - 8);
  const toY = horizontal ? clamp(y, box.top + 8, box.bottom - 8) : (side === "below" ? box.top : box.bottom);

  marker.dataset.side = side;
  drawBox(marker, target);
  line.style.left = `${fromX}px`;
  line.style.top = `${fromY}px`;
  line.style.width = `${Math.hypot(toX - fromX, toY - fromY)}px`;
  line.style.transform = `rotate(${Math.atan2(toY - fromY, toX - fromX)}rad)`;
}
