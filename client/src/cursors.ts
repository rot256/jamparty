// Cursors: this page's pointer, sent while "Show my cursor" is on, and
// other people's, drawn while they are on this page.
//
// Sending happens only on movement: one message per `pointermove`, at most
// CURSOR_RATE per second, the last position of a window always sent, and
// nothing while the pointer is still. Leaving the page sends one message
// without a position. Positions are element-relative: the element's
// selector and fractions of its box, never viewport coordinates.

import { sendCursor } from "./api";
import { cssSelector } from "./capture";
import { CURSOR_FADE_MS, CURSOR_RATE } from "./config";
import type { Person, Pointer, Position } from "./generated/api";
import { TAB_ID } from "./location";
import { colourOf } from "./panel";
import type { Ui } from "./ui";

const FRACTION_ONE = 10000;
const INTERVAL_MS = 1000 / CURSOR_RATE;
const MAX_SELECTOR = 512;

function fraction(offset: number, size: number): number {
  if (size <= 0) return 0;
  return Math.round(Math.min(1, Math.max(0, offset / size)) * FRACTION_ONE);
}

/** What is waiting to go out: a position, or the pointer left (null). */
interface Pending {
  position: Position | null;
}

export class CursorSender {
  private on = false;
  /**
   * The page the joiner confirmed for this tab, and the number of that
   * report; null while it has none (not yet confirmed, being changed, or
   * not representable). Nothing is sent then.
   */
  private page: { url: string; seq: number } | null = null;
  /**
   * A representable page awaits the joiner's confirmation: movement meanwhile
   * is kept (the latest) and sent once it is confirmed.
   */
  private awaiting = false;
  /** The latest pointer position not yet sent. */
  private latest: { x: number; y: number } | null = null;
  private lastSent = 0;
  private timer: number | undefined;
  /** Whether others currently see this pointer (a position went out). */
  private present = false;
  /** At most one request in flight, and one waiting: only the latest. */
  private pending: Pending | null = null;
  private sending = false;

  constructor(private readonly ui: Ui) {
    document.addEventListener("pointermove", (event) => { this.moved(event); }, {
      capture: true,
      passive: true,
    });
    document.documentElement.addEventListener("pointerleave", () => { this.left(); });
    window.addEventListener("blur", () => { this.left(); });
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") this.left();
    });
  }

  private get active(): boolean {
    return this.on && this.page !== null;
  }

  /** Forgets everything not sent yet. */
  private reset(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.latest = null;
    this.pending = null;
    this.present = false;
  }

  /** The cursor switch; nothing at all is sent while it is off. */
  setOn(on: boolean): void {
    this.on = on;
    if (!on) this.reset();
  }

  /**
   * The page is being registered with the joiner: nothing is sent until it
   * is confirmed, but the latest movement is kept for then.
   */
  registering(): void {
    const latest = this.latest;
    this.page = null;
    this.reset();
    this.awaiting = true;
    this.latest = latest;
  }

  /** The joiner took `url` for this tab, with report number `seq`. */
  confirmed(url: string, seq: number): void {
    this.page = { url, seq };
    this.awaiting = false;
    if (this.latest !== null) this.schedule();
  }

  /** No page (not representable, or no event stream): nothing is kept or sent. */
  suspend(): void {
    this.page = null;
    this.awaiting = false;
    this.reset();
  }

  private moved(event: PointerEvent): void {
    if (!this.on || event.pointerType === "touch") return;
    if (this.page === null && !this.awaiting) return;
    this.latest = { x: event.clientX, y: event.clientY };
    if (this.page !== null) this.schedule();
  }

  /** Sends the latest movement now, or at the end of the rate window. */
  private schedule(): void {
    if (this.timer !== undefined) return;
    const wait = this.lastSent + INTERVAL_MS - performance.now();
    if (wait <= 0) {
      this.flush();
    } else {
      this.timer = window.setTimeout(() => {
        this.timer = undefined;
        this.flush();
      }, wait);
    }
  }

  private flush(): void {
    const latest = this.latest;
    this.latest = null;
    if (latest === null || !this.active) return;
    const position = this.position(latest.x, latest.y);
    if (position === null) return;
    this.lastSent = performance.now();
    this.present = true;
    this.enqueue(position);
  }

  /** The pointer left the page: one message without a position. */
  private left(): void {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    this.latest = null;
    if (!this.active || !this.present) return;
    this.present = false;
    this.enqueue(null);
  }

  /** The element under the point (not jampot's own overlay) and the offset in its box. */
  private position(x: number, y: number): Position | null {
    const element = document
      .elementsFromPoint(x, y)
      .find((candidate) => candidate !== this.ui.host && !this.ui.host.contains(candidate));
    if (element === undefined) return null;
    const selector = cssSelector(element);
    if (selector.length > MAX_SELECTOR) return null;
    const rect = element.getBoundingClientRect();
    return {
      selector,
      x: fraction(x - rect.left, rect.width),
      y: fraction(y - rect.top, rect.height),
    };
  }

  /** Replaces whatever waits (a leave replaces positions too). */
  private enqueue(position: Position | null): void {
    this.pending = { position };
    void this.pump();
  }

  private async pump(): Promise<void> {
    if (this.sending) return;
    this.sending = true;
    for (let next = this.pending; next !== null; next = this.pending) {
      this.pending = null;
      // The switch and the page are checked right before each send.
      const page = this.page;
      if (!this.on || page === null) break;
      const { url, seq } = page;
      const pointer: Pointer = next.position === null
        ? { tab: TAB_ID, seq, url }
        : { tab: TAB_ID, seq, url, position: next.position };
      await sendCursor(pointer);
    }
    this.sending = false;
  }
}

interface Drawn {
  node: HTMLDivElement;
  tag: HTMLSpanElement;
  arrow: SVGPathElement;
  element: Element;
  x: number;
  y: number;
  /** Where it is drawn now (it glides towards the element's point). */
  shownX: number | null;
  shownY: number | null;
  updated: number;
}

const SVG_NS = "http://www.w3.org/2000/svg";

export class CursorLayer {
  private readonly drawn = new Map<string, Drawn>();
  private people = new Map<string, Person>();
  private frame: number | undefined;

  constructor(private readonly ui: Ui) {}

  /** Someone's pointer on this page moved, or (null) left it. */
  update(key: string, position: Position | null): void {
    if (position === null) {
      this.remove(key);
      return;
    }
    let element: Element | null;
    try {
      element = document.querySelector(position.selector);
    } catch {
      element = null;
    }
    // Drawn only where the selector means something here.
    if (element === null || element === this.ui.host || this.ui.host.contains(element)) {
      this.remove(key);
      return;
    }
    const drawn = this.drawn.get(key) ?? this.create(key);
    drawn.element = element;
    drawn.x = position.x / FRACTION_ONE;
    drawn.y = position.y / FRACTION_ONE;
    drawn.updated = performance.now();
    drawn.node.dataset.faded = "false";
    this.paint(drawn);
    this.schedule();
  }

  /** Who is here: cursors of people gone, elsewhere or switched off go. */
  setPeople(here: readonly Person[], url: string | null): void {
    this.people = new Map(here.map((person) => [person.key, person]));
    for (const key of [...this.drawn.keys()]) {
      const person = this.people.get(key);
      if (person?.cursor !== "on" || url === null || person.url !== url) {
        this.remove(key);
      }
    }
    for (const drawn of this.drawn.values()) this.paint(drawn);
  }

  clear(): void {
    for (const key of [...this.drawn.keys()]) this.remove(key);
  }

  get keys(): string[] {
    return [...this.drawn.keys()];
  }

  private remove(key: string): void {
    this.drawn.get(key)?.node.remove();
    this.drawn.delete(key);
  }

  private create(key: string): Drawn {
    const node = document.createElement("div");
    node.className = "cursor";
    node.dataset.key = key;
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("width", "16");
    svg.setAttribute("height", "18");
    svg.setAttribute("viewBox", "0 0 16 18");
    const arrow = document.createElementNS(SVG_NS, "path");
    arrow.setAttribute("d", "M1.5 1.5 L1.5 14.5 L5.2 11.2 L7.8 16.5 L10.2 15.4 L7.7 10.3 L12.8 10.3 Z");
    arrow.setAttribute("stroke", "#ffffff");
    arrow.setAttribute("stroke-width", "1.4");
    arrow.setAttribute("stroke-linejoin", "round");
    svg.append(arrow);
    const tag = document.createElement("span");
    tag.className = "tag";
    node.append(svg, tag);
    this.ui.cursors.append(node);
    const drawn: Drawn = {
      node,
      tag,
      arrow,
      element: document.documentElement,
      x: 0,
      y: 0,
      shownX: null,
      shownY: null,
      updated: performance.now(),
    };
    this.drawn.set(key, drawn);
    return drawn;
  }

  private paint(drawn: Drawn): void {
    const person = this.people.get(drawn.node.dataset.key ?? "");
    const colour = person === undefined ? "#555" : colourOf(person);
    drawn.arrow.setAttribute("fill", colour);
    drawn.tag.style.background = colour;
    drawn.tag.textContent = person?.name ?? "";
  }

  private schedule(): void {
    if (this.frame !== undefined) return;
    this.frame = requestAnimationFrame(() => { this.animate(); });
  }

  /** Glides each cursor towards its point (kept on its element as the page scrolls) and fades idle ones. */
  private animate(): void {
    this.frame = undefined;
    const now = performance.now();
    let moving = false;
    for (const drawn of this.drawn.values()) {
      if (!drawn.element.isConnected) {
        drawn.node.dataset.faded = "true";
        continue;
      }
      const rect = drawn.element.getBoundingClientRect();
      const targetX = rect.left + drawn.x * rect.width;
      const targetY = rect.top + drawn.y * rect.height;
      const shownX = drawn.shownX === null ? targetX : drawn.shownX + (targetX - drawn.shownX) * 0.35;
      const shownY = drawn.shownY === null ? targetY : drawn.shownY + (targetY - drawn.shownY) * 0.35;
      drawn.shownX = Math.abs(targetX - shownX) < 0.5 ? targetX : shownX;
      drawn.shownY = Math.abs(targetY - shownY) < 0.5 ? targetY : shownY;
      drawn.node.style.transform = `translate(${drawn.shownX}px, ${drawn.shownY}px)`;
      const idle = now - drawn.updated > CURSOR_FADE_MS;
      drawn.node.dataset.faded = String(idle);
      // Keep animating while gliding, and until idle ones have faded.
      if (!idle) moving = true;
    }
    if (moving) this.schedule();
  }
}
