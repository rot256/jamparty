// Keep answer cards outside their targets, even when the viewport is narrow.
export interface Box {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

export type Side = "right" | "left" | "below" | "above";

export const oppositeSide: Readonly<Record<Side, Side>> = {
  right: "left", left: "right", above: "below", below: "above",
};

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
const TARGET_GAP = 28;
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

export function clamp(value: number, lower: number, upper: number): number {
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
  preferredSide?: Side, scale = 1,
): Placement | undefined {
  const bands: Band[] = [
    ...(page === undefined ? [] : sideBands(view, page, PAGE_GAP)),
    ...sideBands(view, target, TARGET_GAP),
    { side: "below", area: { ...view, top: Math.max(view.top, target.bottom + TARGET_GAP) } },
    { side: "above", area: { ...view, bottom: Math.min(view.bottom, target.top - TARGET_GAP) } },
  ];
  const passes = preferredSide === undefined ? [false, true] : [preferredSide, false, true];
  for (const pass of passes) {
    const allowClipping = pass === true;
    let margin: { placement: Placement; score: number } | undefined;
    for (const [index, { side, area }] of bands.entries()) {
      if (typeof pass === "string" && side !== pass) continue;
      if (index === 2 && page !== undefined && margin !== undefined) return margin.placement;
      const width = Math.min(MAX_WIDTH * scale, Math.floor(area.right - area.left));
      if (width < Math.min(MIN_WIDTH, Math.floor(view.right - view.left)) || width <= 0) continue;
      const height = heightAt(width);
      const horizontal = side === "right" || side === "left";
      const targetCenter = (target.left + target.right) / 2;
      const desiredLeft = clamp(horizontal ? target.left : targetCenter - width / 2, area.left, area.right - width);
      const lefts = horizontal
        ? [side === "right" ? area.left : area.right - width]
        : [desiredLeft, ...occupied.flatMap((box) => [box.right + GAP, box.left - width - GAP])]
          .filter((left) => left >= area.left && left + width <= area.right);
      let best: { placement: Placement; score: number } | undefined;
      for (const left of lefts) {
        const obstacles = occupied.filter((box) => left < box.right + GAP && left + width > box.left - GAP);
        const desiredTop = side === "above" ? target.top - TARGET_GAP - height : target.top;
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
      if (best === undefined) continue;
      if (side === pass || page === undefined || index >= 2) return best.placement;
      if (margin === undefined || best.score < margin.score) margin = best;
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

export const SVG_NS = "http://www.w3.org/2000/svg";

export function nearestSide(target: Box, card: Box): Side {
  const dx = (card.left + card.right - target.left - target.right) / 2;
  const dy = (card.top + card.bottom - target.top - target.bottom) / 2;
  const horizontalGap = Math.max(card.left - target.right, target.left - card.right);
  const verticalGap = Math.max(card.top - target.bottom, target.top - card.bottom);
  if (horizontalGap < 0 && verticalGap < 0 ? Math.abs(dx) >= Math.abs(dy) : horizontalGap >= verticalGap) {
    return dx >= 0 ? "right" : "left";
  }
  return dy >= 0 ? "below" : "above";
}

export function drawAnswerLink(
  marker: HTMLElement, link: SVGGElement, target: Box, { side, box }: Placement,
): void {
  const horizontal = side === "right" || side === "left";
  const sign = side === "right" || side === "below" ? 1 : -1;
  const edge = { right: target.right, left: target.left, below: target.bottom, above: target.top }[side];
  const start = horizontal ? target.top : target.left;
  const end = horizontal ? target.bottom : target.right;
  const span = end - start;
  const mid = (start + end) / 2;
  const base = edge + sign * (horizontal ? 7 : 4);
  const destination = { right: box.left, left: box.right, below: box.top, above: box.bottom }[side];
  const depth = Math.min(horizontal ? 9 : 5,
    Math.max(2, (destination - base) * sign - 2), Math.max(4, span / 5));
  // Express the brace in outward/along coordinates, then rotate or mirror
  // the same four cubics onto the target edge facing the card.
  const point = (outward: number, along: number): string =>
    horizontal ? `${outward},${along}` : `${along},${outward}`;
  const waist = base + sign * depth / 2;
  const tip = base + sign * depth;
  const [brace, curve, dot] = link.children;
  brace?.setAttribute("d", [
    `M ${point(base, start)}`,
    `C ${point(waist, start)} ${point(waist, start + span * .14)} ${point(waist, start + span * .25)}`,
    `C ${point(waist, mid - span * .08)} ${point(waist, mid)} ${point(tip, mid)}`,
    `C ${point(waist, mid)} ${point(waist, mid + span * .08)} ${point(waist, end - span * .25)}`,
    `C ${point(waist, end - span * .14)} ${point(waist, end)} ${point(base, end)}`,
  ].join(" "));

  const near = horizontal ? box.top : box.left;
  const far = horizontal ? box.bottom : box.right;
  // Keep the dot clear of the rounded corners as the card moves.
  const inset = (far - near) / 4;
  const along = clamp(mid, near + inset, far - inset);
  const bend = (tip + destination) / 2;
  curve?.setAttribute("d", `M ${point(tip, mid)} C ${point(bend, mid)} ${point(bend, along)} ${point(destination, along)}`);
  dot?.setAttribute("cx", String(horizontal ? destination : along));
  dot?.setAttribute("cy", String(horizontal ? along : destination));
  marker.dataset.side = side;
  drawBox(marker, target);
}
