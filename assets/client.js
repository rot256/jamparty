// Generated from client/src by `npm run build`; do not edit.

// client/src/json.ts
function isRecord(value) {
  return typeof value === "object" && value !== null;
}

// client/src/pdf-anchor.ts
function parsePdfAnchor(value) {
  if (!isRecord(value) || typeof value.page !== "number" || !Number.isInteger(value.page) || value.page < 1 || value.page > 4294967295 || !isRecord(value.box)) return void 0;
  const { x0, y0, x1, y1 } = value.box;
  if (typeof x0 !== "number" || typeof y0 !== "number" || typeof x1 !== "number" || typeof y1 !== "number" || !(0 <= x0 && x0 < x1 && x1 <= 14400 && 0 <= y0 && y0 < y1 && y1 <= 14400)) {
    return void 0;
  }
  return { page: value.page, box: { x0, y0, x1, y1 } };
}

// client/src/answer-anchor.ts
function validPath(path) {
  if (path.length > 4096 || !path.startsWith("/") || path.startsWith("//") || /[^\x21-\x7e]|["\\?#]/u.test(path)) return false;
  try {
    const decoded = decodeURIComponent(path);
    return new URL(path, "http://localhost").pathname === path && !decoded.startsWith("//") && !decoded.includes("\\") && !Array.from(decoded).some((character) => /\p{Cc}/u.test(character)) && !decoded.split("/").some((part) => part === "." || part === "..") && decoded.split("/")[1] !== "_jamparty";
  } catch {
    return false;
  }
}
function parsePlaceAnchor(value) {
  if (!isRecord(value) || Object.keys(value).length !== 3 || typeof value.path !== "string" || !validPath(value.path)) return void 0;
  const { path } = value;
  if (value.type === "html" && typeof value.selector === "string" && value.selector.trim() !== "" && Array.from(value.selector).length <= 2048) {
    return { type: "html", path, selector: value.selector };
  }
  if (value.type === "pdf" && path.toLowerCase().endsWith(".pdf") && isRecord(value.pdf) && Object.keys(value.pdf).length === 2 && isRecord(value.pdf.box) && Object.keys(value.pdf.box).length === 4) {
    const pdf = parsePdfAnchor(value.pdf);
    if (pdf !== void 0) return { type: "pdf", path, pdf };
  }
  return void 0;
}
function placeElement(root, selector) {
  try {
    return root.querySelector(selector);
  } catch {
    return null;
  }
}

// client/src/answer-placement.ts
var oppositeSide = {
  right: "left",
  left: "right",
  above: "below",
  below: "above"
};
var EDGE = 8;
var GAP = 12;
var TARGET_GAP = 28;
var PAGE_GAP = 24;
var MIN_WIDTH = 200;
var MAX_WIDTH = 340;
var MIN_HEIGHT = 48;
function answerViewport() {
  const viewport = window.visualViewport;
  const x = viewport?.offsetLeft ?? 0;
  const y = viewport?.offsetTop ?? 0;
  return {
    left: x + EDGE,
    top: y + EDGE,
    right: x + (viewport?.width ?? window.innerWidth) - EDGE,
    bottom: y + (viewport?.height ?? window.innerHeight) - EDGE
  };
}
function overlaps(a, b) {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}
function clamp(value, lower, upper) {
  return Math.max(lower, Math.min(value, upper));
}
function sideBands(view, target, gap) {
  const right = { ...view, left: Math.max(view.left, target.right + gap) };
  const left = { ...view, right: Math.min(view.right, target.left - gap) };
  const bands = [{ side: "right", area: right }, { side: "left", area: left }];
  return bands.sort((a, b) => b.area.right - b.area.left - (a.area.right - a.area.left));
}
function placeAnswer(heightAt, view, target, page, occupied, preferredSide, scale = 1) {
  const bands = [
    ...page === void 0 ? [] : sideBands(view, page, PAGE_GAP),
    ...sideBands(view, target, TARGET_GAP),
    { side: "below", area: { ...view, top: Math.max(view.top, target.bottom + TARGET_GAP) } },
    { side: "above", area: { ...view, bottom: Math.min(view.bottom, target.top - TARGET_GAP) } }
  ];
  const passes = preferredSide === void 0 ? [false, true] : [preferredSide, false, true];
  for (const pass of passes) {
    const allowClipping = pass === true;
    let margin;
    for (const [index, { side, area }] of bands.entries()) {
      if (typeof pass === "string" && side !== pass) continue;
      if (index === 2 && page !== void 0 && margin !== void 0) return margin.placement;
      const width = Math.min(MAX_WIDTH * scale, Math.floor(area.right - area.left));
      if (width < Math.min(MIN_WIDTH, Math.floor(view.right - view.left)) || width <= 0) continue;
      const height = heightAt(width);
      const horizontal = side === "right" || side === "left";
      const targetCenter = (target.left + target.right) / 2;
      const desiredLeft = clamp(horizontal ? target.left : targetCenter - width / 2, area.left, area.right - width);
      const lefts = horizontal ? [side === "right" ? area.left : area.right - width] : [desiredLeft, ...occupied.flatMap((box) => [box.right + GAP, box.left - width - GAP])].filter((left) => left >= area.left && left + width <= area.right);
      let best;
      for (const left of lefts) {
        const obstacles = occupied.filter((box) => left < box.right + GAP && left + width > box.left - GAP);
        const desiredTop = side === "above" ? target.top - TARGET_GAP - height : target.top;
        const tops = [
          clamp(desiredTop, area.top, area.bottom - height),
          ...obstacles.flatMap((box) => [box.bottom + GAP, box.top - GAP - height]),
          area.top
        ];
        for (const top of tops) {
          if (top < area.top || top >= area.bottom) continue;
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
          const score = Math.abs(top - desiredTop) + Math.abs(left - desiredLeft) + (height - fittedHeight) * 2;
          if (best === void 0 || score < best.score) {
            best = {
              placement: { side, maxHeight, box: { left, right: left + width, top, bottom: top + fittedHeight } },
              score
            };
          }
        }
      }
      if (best === void 0) continue;
      if (side === pass || page === void 0 || index >= 2) return best.placement;
      if (margin === void 0 || best.score < margin.score) margin = best;
    }
  }
  return void 0;
}
function drawBox(node, box) {
  node.style.left = `${box.left}px`;
  node.style.top = `${box.top}px`;
  node.style.width = `${box.right - box.left}px`;
  node.style.height = `${box.bottom - box.top}px`;
}
var SVG_NS = "http://www.w3.org/2000/svg";
function nearestSide(target, card) {
  const dx = (card.left + card.right - target.left - target.right) / 2;
  const dy = (card.top + card.bottom - target.top - target.bottom) / 2;
  const horizontalGap = Math.max(card.left - target.right, target.left - card.right);
  const verticalGap = Math.max(card.top - target.bottom, target.top - card.bottom);
  if (horizontalGap < 0 && verticalGap < 0 ? Math.abs(dx) >= Math.abs(dy) : horizontalGap >= verticalGap) {
    return dx >= 0 ? "right" : "left";
  }
  return dy >= 0 ? "below" : "above";
}
function drawAnswerLink(marker, link, target, { side, box }) {
  const horizontal = side === "right" || side === "left";
  const sign = side === "right" || side === "below" ? 1 : -1;
  const edge = { right: target.right, left: target.left, below: target.bottom, above: target.top }[side];
  const start = horizontal ? target.top : target.left;
  const end = horizontal ? target.bottom : target.right;
  const span = end - start;
  const mid = (start + end) / 2;
  const base = edge + sign * (horizontal ? 7 : 4);
  const destination = { right: box.left, left: box.right, below: box.top, above: box.bottom }[side];
  const depth = Math.min(
    horizontal ? 9 : 5,
    Math.max(2, (destination - base) * sign - 2),
    Math.max(4, span / 5)
  );
  const point = (outward, along2) => horizontal ? `${outward},${along2}` : `${along2},${outward}`;
  const waist = base + sign * depth / 2;
  const tip = base + sign * depth;
  const [brace, curve, dot] = link.children;
  brace?.setAttribute("d", [
    `M ${point(base, start)}`,
    `C ${point(waist, start)} ${point(waist, start + span * 0.14)} ${point(waist, start + span * 0.25)}`,
    `C ${point(waist, mid - span * 0.08)} ${point(waist, mid)} ${point(tip, mid)}`,
    `C ${point(waist, mid)} ${point(waist, mid + span * 0.08)} ${point(waist, end - span * 0.25)}`,
    `C ${point(waist, end - span * 0.14)} ${point(waist, end)} ${point(base, end)}`
  ].join(" "));
  const near = horizontal ? box.top : box.left;
  const far = horizontal ? box.bottom : box.right;
  const inset = (far - near) / 4;
  const along = clamp(mid, near + inset, far - inset);
  const bend = (tip + destination) / 2;
  curve?.setAttribute("d", `M ${point(tip, mid)} C ${point(bend, mid)} ${point(bend, along)} ${point(destination, along)}`);
  dot?.setAttribute("cx", String(horizontal ? destination : along));
  dot?.setAttribute("cy", String(horizontal ? along : destination));
  marker.dataset.side = side;
  drawBox(marker, target);
}

// client/src/generated/api.ts
var MAX_NAME = 40;
var MAX_ANSWER_TIMEOUT = 3600;
var DEFAULT_ANSWER_TIMEOUT = 30;
var MAX_COMMENT = 1e4;
var MAX_FILE_BYTES = 8388608;
var MAX_ATTACHMENTS = 4;
var MAX_SELECTED_TEXT = 2e3;
var MAX_PAGE_PATH = 512;
var MAX_CURSOR_SELECTOR = 512;
var FRACTION_ONE = 1e4;
var MAX_ANSWERS = 100;
var MAX_JOINERS = 64;
var DEFAULT_CLEAR_ANSWERS_KEY = "d";
var RESERVED_KEYS = ["/", "Escape"];
var MODIFIER_KEYS = ["Alt", "AltGraph", "CapsLock", "Control", "Fn", "FnLock", "Hyper", "Meta", "NumLock", "ScrollLock", "Shift", "Super", "Symbol"];
var PAGE_KEYS = ["Enter", "Tab", " "];

// client/src/capture.ts
function truncate(value, length) {
  return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
}
function normalizeText(value) {
  return value.replace(/\s+/g, " ").trim();
}
function commentText(value) {
  const selected = normalizeText(value);
  return selected ? truncate(selected, MAX_SELECTED_TEXT) : null;
}
function selectedTextFor(target) {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  try {
    if (!selection.getRangeAt(0).intersectsNode(target)) return null;
  } catch {
    return null;
  }
  return commentText(selection.toString());
}
function uniqueIdSelector(element) {
  if (!element.id) return null;
  const selector = `#${CSS.escape(element.id)}`;
  return document.querySelectorAll(selector).length === 1 ? selector : null;
}
function cssSelector(target) {
  const parts = [];
  let element = target;
  while (element) {
    const idSelector = uniqueIdSelector(element);
    if (idSelector) {
      parts.unshift(idSelector);
      break;
    }
    let part = element.tagName.toLowerCase();
    const parent = element.parentElement;
    if (parent) {
      const tagName = element.tagName;
      const sameTag = Array.from(parent.children).filter(
        (sibling) => sibling.tagName === tagName
      );
      if (sameTag.length > 1) part += `:nth-of-type(${sameTag.indexOf(element) + 1})`;
    }
    parts.unshift(part);
    element = parent;
  }
  return parts.join(" > ");
}
function captureTarget(target, selector = cssSelector(target)) {
  return {
    element: target,
    page: window.location.href,
    selector,
    selectedText: selectedTextFor(target)
  };
}
function capturePageTarget() {
  return {
    element: null,
    page: window.location.href,
    selector: "",
    selectedText: selectedTextFor(document.documentElement)
  };
}
function samePage(page) {
  if (page === void 0) return true;
  try {
    const url = new URL(page);
    return url.pathname === window.location.pathname && url.search === window.location.search;
  } catch {
    return false;
  }
}

// client/src/drag.ts
function fitPosition({ x, y }, width, height) {
  const view = answerViewport();
  return {
    x: clamp(x, view.left, view.right - width),
    y: clamp(y, view.top, view.bottom - height)
  };
}
var directions = {
  ArrowLeft: { x: -1, y: 0 },
  ArrowRight: { x: 1, y: 0 },
  ArrowUp: { x: 0, y: -1 },
  ArrowDown: { x: 0, y: 1 }
};
function moveBy(handle, surface, move, savePlacement, anchorOrigin, fit = fitPosition) {
  const button = document.createElement("button");
  button.className = "move-control";
  button.type = "button";
  button.setAttribute("aria-label", "Move");
  button.title = "Move with arrow keys";
  button.innerHTML = '<svg aria-hidden="true" width="12" height="14" viewBox="0 0 12 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 3h0m4 0h0M4 7h0m4 0h0m-4 4h0m4 0h0"/></svg>';
  handle.prepend(button);
  let drag;
  let keyboard;
  let restore;
  const end = () => {
    const id = drag?.id;
    drag = void 0;
    keyboard = void 0;
    restore = void 0;
    delete handle.dataset.dragging;
    window.removeEventListener("keydown", escape, true);
    if (id !== void 0 && handle.hasPointerCapture(id)) handle.releasePointerCapture(id);
  };
  function escape(event) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopImmediatePropagation();
    restore?.();
    end();
  }
  const begin = () => {
    restore = savePlacement();
    window.addEventListener("keydown", escape, true);
  };
  handle.addEventListener("pointerdown", (event) => {
    const control = event.target instanceof Element ? event.target.closest("button, a") : null;
    if (!event.isPrimary || event.button !== 0 || drag !== void 0 || control !== null && control !== button) return;
    if (control === button) button.focus({ preventScroll: true });
    end();
    drag = {
      id: event.pointerId,
      start: { x: event.clientX, y: event.clientY },
      rect: surface.getBoundingClientRect(),
      moved: false
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
  const moveWithKeys = (event) => {
    if (drag !== void 0 || event.isComposing || event.altKey || event.ctrlKey || event.metaKey || !event.composedPath().includes(button)) return;
    const direction = Object.hasOwn(directions, event.key) ? directions[event.key] : void 0;
    if (direction === void 0 && (event.key !== "Enter" || keyboard === void 0)) return;
    event.preventDefault();
    event.stopPropagation();
    if (direction === void 0) {
      end();
      return;
    }
    const origin = anchorOrigin?.();
    if (keyboard === void 0) {
      const rect2 = surface.getBoundingClientRect();
      keyboard = { point: { x: rect2.left - (origin?.x ?? 0), y: rect2.top - (origin?.y ?? 0) }, rect: rect2 };
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
    move(applied, rect.width, (point2) => {
      if (keyboard === active) keyboard.point = point2;
    });
  };
  const stopListening = () => {
    window.removeEventListener("keydown", moveWithKeys, true);
    if (keyboard !== void 0) end();
  };
  button.addEventListener("focus", () => {
    window.addEventListener("keydown", moveWithKeys, true);
  });
  button.addEventListener("blur", stopListening);
  return () => {
    stopListening();
    end();
  };
}

// client/src/config.ts
var SCRIPT_URL = new URL(import.meta.url);
var BASE = new URL("./", SCRIPT_URL);
var SOCKET_URL = (() => {
  const url = new URL("api/socket", BASE);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url;
})();
var LIVE_REVISION = (() => {
  const live = SCRIPT_URL.searchParams.get("live");
  if (live === null || !/^\d+$/.test(live)) return null;
  const revision = Number(live);
  return Number.isSafeInteger(revision) ? revision : null;
})();
var UPLOAD_URL = new URL("api/attachments", BASE);
var HOST_ID = "jamparty";
var DISMISSED_KEY = "jamparty-dismissed-v1";
var OUTBOX_KEY = "jamparty-outbox-v1";
var JOINER_KEY = "jamparty-joiner-key-v1";
var DRAFT_KEY = "jamparty-draft-v1";
var EDIT_REF_KEY = "jamparty-edit-ref-v1";
var SETTINGS_COOKIE = "jamparty-settings-v1";
var MAX_OUTBOX = 20;
var WATCHDOG_MS = 35e3;
var RECONNECT_MIN_MS = 250;
var RECONNECT_MAX_MS = 5e3;
var AGENT_TARGET_PREFIX = "jamparty-agent:";
var CURSOR_RATE = 15;
var CURSOR_FADE_MS = 4e3;
var PALETTE = [
  "#0b7896",
  "#9c4f96",
  "#b4461f",
  "#2b7a3d",
  "#5f5bb8",
  "#a33a5b",
  "#7a6200",
  "#3d6b8f"
];

// client/src/answers.ts
function translate(box, x, y) {
  return { left: box.left + x, right: box.right + x, top: box.top + y, bottom: box.bottom + y };
}
function sameBox(a, b) {
  if (a === void 0 || b === void 0) return a === b;
  return a.left === b.left && a.top === b.top && a.right === b.right && a.bottom === b.bottom;
}
function intersects(a, b) {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}
function containsFixed(style) {
  return [
    style.transform,
    style.translate,
    style.rotate,
    style.scale,
    style.perspective,
    style.filter,
    style.backdropFilter
  ].some((value) => value !== "" && value !== "none") || /\b(layout|paint|strict|content)\b/.test(style.contain) || style.contentVisibility === "auto" || /\b(transform|translate|rotate|scale|perspective|filter|backdrop-filter|contain|content-visibility)\b/.test(style.willChange);
}
function bodyScrolls() {
  const root = getComputedStyle(document.documentElement);
  const body = getComputedStyle(document.body);
  return [root.overflowX, root.overflowY].some((overflow) => overflow !== "visible") && [body.overflowX, body.overflowY].some((overflow) => /^(auto|scroll|hidden)$/.test(overflow));
}
function scrollRelationship(target, rect) {
  const body = bodyScrolls();
  const sources = /* @__PURE__ */ new Set();
  let fixed = false;
  let sticky = false;
  for (let node = target; node !== null; node = node.parentElement) {
    const style = getComputedStyle(node);
    if (fixed && !containsFixed(style)) continue;
    if (node !== target && (node !== document.body || body) && node !== document.documentElement && [style.overflowX, style.overflowY].some((overflow) => /^(auto|scroll|hidden)$/.test(overflow))) {
      sources.add(node);
    }
    fixed = style.position === "fixed";
    sticky ||= style.position === "sticky";
  }
  if (fixed) return { mode: sources.size > 0 ? "tracked" : "fixed", sources, target, rect };
  if (!sticky && sources.size === 0) return { mode: "document", sources, target, rect };
  sources.add(document);
  return { mode: "tracked", sources, target, rect };
}
function makeLink() {
  const link = document.createElementNS(SVG_NS, "g");
  const brace = document.createElementNS(SVG_NS, "path");
  brace.classList.add("agent-anchor-brace");
  const curve = document.createElementNS(SVG_NS, "path");
  curve.classList.add("agent-anchor-line");
  const dot = document.createElementNS(SVG_NS, "circle");
  dot.setAttribute("r", "2.3");
  link.append(brace, curve, dot);
  return link;
}
var REVEAL_KEY = "jamparty-answer-reveal";
var REVEAL_TTL_MS = 3e4;
function takeReveal() {
  try {
    const stored = sessionStorage.getItem(REVEAL_KEY);
    sessionStorage.removeItem(REVEAL_KEY);
    const parsed = stored === null ? null : JSON.parse(stored);
    if (typeof parsed !== "object" || parsed === null) return null;
    const { id, at } = parsed;
    if (typeof id !== "string" || typeof at !== "number") return null;
    const age = Date.now() - at;
    return age >= 0 && age <= REVEAL_TTL_MS ? id : null;
  } catch {
    return null;
  }
}
function loadDismissed() {
  try {
    const stored = JSON.parse(sessionStorage.getItem(DISMISSED_KEY) ?? "[]");
    const ids = Array.isArray(stored) ? stored.filter((id) => typeof id === "string") : [];
    return new Set(ids.slice(-MAX_ANSWERS));
  } catch {
    return /* @__PURE__ */ new Set();
  }
}
var Answers = class {
  constructor(ui, pick, pdf) {
    this.ui = ui;
    this.pick = pick;
    this.pdf = pdf;
    const schedule = () => {
      this.schedulePlacement();
    };
    this.geometry = new ResizeObserver(schedule);
    this.geometry.observe(ui.list);
    this.geometry.observe(ui.bar);
    this.geometry.observe(ui.panel);
    this.geometry.observe(ui.agentGeneral);
    this.geometry.observe(ui.dialog);
    this.obstacleChanges = new MutationObserver(schedule);
    for (const node of [ui.list, ui.bar, ui.panel, ui.dialog]) {
      this.obstacleChanges.observe(node, { attributes: true });
    }
    window.visualViewport?.addEventListener("resize", schedule);
    window.visualViewport?.addEventListener("scroll", this.onScroll);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", this.onScroll, { capture: true, passive: true });
    window.addEventListener("load", schedule, true);
    const indicator = this.pdf === void 0 ? null : document.querySelector("#pdf-position");
    new MutationObserver((records) => {
      if (records.some(({ target }) => target !== indicator)) {
        schedule();
      }
    }).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true
    });
  }
  ui;
  pick;
  pdf;
  shown = /* @__PURE__ */ new Map();
  sections = null;
  places = /* @__PURE__ */ new Map();
  historyPreview;
  recent = /* @__PURE__ */ new Map();
  onHistory = () => void 0;
  historyTarget = () => null;
  dismissed = loadDismissed();
  placementFrame;
  scrollFrame;
  scrolled = /* @__PURE__ */ new Set();
  obstacleChanges;
  timeoutSeconds = 0;
  outlined;
  highlightTimer;
  geometry;
  visibleTargets = /* @__PURE__ */ new Map();
  visibility = new IntersectionObserver((entries) => {
    for (const { target, isIntersecting: visible } of entries) {
      this.visibleTargets.set(target, visible);
      for (const entry of this.shown.values()) {
        if (entry.placed?.target === target) entry.wrapper.classList.toggle("target-offscreen", !visible);
      }
    }
  });
  observedTargets = /* @__PURE__ */ new Set();
  pendingReveal = takeReveal();
  landingPath = location.pathname;
  /** The node of answer `id`, for comments on an answer. */
  node(id) {
    return this.shown.get(id)?.node ?? null;
  }
  /** The element a selector names: on the page, or one of the answers. */
  targetFor(selector) {
    if (selector.startsWith(AGENT_TARGET_PREFIX)) {
      const id = selector.slice(AGENT_TARGET_PREFIX.length);
      return this.node(id) ?? (this.dismissed.has(id) ? null : this.historyTarget(id));
    }
    return document.querySelector(selector);
  }
  /** A new or reconnected tab sees cached replies quietly. */
  sync(answers) {
    this.recent.clear();
    for (const answer of answers) this.recent.set(answer.id, answer);
    for (const id of this.shown.keys()) if (!this.recent.has(id)) this.remove(id);
    let changed = false;
    for (const id of this.dismissed) {
      if (!this.recent.has(id)) {
        this.dismissed.delete(id);
        changed = true;
      }
    }
    if (changed) this.persistDismissed();
    for (const answer of answers) {
      if (answer.choices.length > 0) this.show(answer);
    }
    this.publishHistory();
  }
  receive(answer) {
    if (this.recent.has(answer.id)) return;
    this.recent.set(answer.id, answer);
    if (this.recent.size > MAX_ANSWERS) {
      const oldest = this.recent.keys().next();
      if (!oldest.done) this.recent.delete(oldest.value);
    }
    if (answer.live || answer.choices.length > 0) this.show(answer);
    this.publishHistory();
  }
  publishHistory() {
    this.onHistory(Array.from(this.recent.values()).filter((answer) => answer.choices.length === 0));
    this.prunePlaces();
    this.schedulePlacement();
  }
  renderBody(answer) {
    const body = document.createElement("div");
    body.className = "agent-body";
    body.innerHTML = answer.html;
    for (const link of body.querySelectorAll("a")) {
      const href = link.getAttribute("href") ?? "";
      if (!href.toLowerCase().startsWith("jamparty:")) {
        link.target = "_blank";
        link.rel = "noopener noreferrer";
      }
    }
    body.addEventListener("click", (event) => {
      const link = event.target instanceof Element ? event.target.closest("a") : null;
      if (link) this.followElementLink(link, event);
    });
    return body;
  }
  placeTarget(answer) {
    const existing = this.places.get(answer.id);
    if (existing !== void 0) return existing;
    const place = parsePlaceAnchor(answer.place);
    if (place === void 0) return void 0;
    const redirectedPath = answer.id === this.pendingReveal ? this.landingPath : void 0;
    const here = () => place.path === location.pathname || redirectedPath === location.pathname;
    const marker = place.type === "pdf" && this.pdf !== void 0 && here() ? document.createElement("div") : void 0;
    let detach = () => void 0;
    if (marker !== void 0 && place.type === "pdf" && this.pdf !== void 0) {
      marker.className = "pdf-answer-anchor";
      detach = this.pdf.trackAnswer(marker, place.pdf);
    }
    const target = () => {
      if (!here()) return void 0;
      const element = place.type === "pdf" ? marker : placeElement(document, place.selector);
      return element?.isConnected && element.getClientRects().length > 0 ? element : void 0;
    };
    const entry = { place, here, target, detach };
    this.places.set(answer.id, entry);
    return entry;
  }
  prunePlaces() {
    for (const [id, entry] of this.places) {
      if (this.recent.has(id) || this.shown.has(id)) continue;
      entry.detach();
      this.places.delete(id);
    }
  }
  showLink(id, entry) {
    const link = document.createElement("a");
    link.className = "agent-show";
    link.textContent = "Show";
    link.href = new URL(entry.place.path, location.origin).href;
    link.addEventListener("click", (event) => {
      if (event.defaultPrevented || event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
      if (entry.here()) {
        event.preventDefault();
        const target = entry.target();
        if (target === void 0) this.ui.showToast("Target unavailable");
        else this.revealElement(target);
        return;
      }
      try {
        sessionStorage.setItem(REVEAL_KEY, JSON.stringify({ id, at: Date.now() }));
      } catch {
      }
    });
    return link;
  }
  renderHistory(answer, node) {
    node.append(this.renderBody(answer));
    const entry = this.placeTarget(answer);
    if (entry === void 0) return;
    node.append(this.showLink(answer.id, entry));
    const preview = () => {
      this.historyPreview = { id: answer.id, node };
      this.schedulePlacement();
    };
    node.addEventListener("pointerenter", preview);
    node.addEventListener("focusin", preview);
    node.addEventListener("pointerleave", () => {
      this.schedulePlacement();
    });
    node.addEventListener("focusout", () => {
      this.schedulePlacement();
    });
  }
  show(answer) {
    if (this.dismissed.has(answer.id) || this.shown.has(answer.id)) return;
    for (const id of this.shown.keys()) {
      if (this.shown.size < MAX_ANSWERS) break;
      this.remove(id);
    }
    const node = document.createElement("article");
    node.className = "agent-message";
    node.dataset.messageId = answer.id;
    const close = document.createElement("button");
    close.className = "agent-dismiss";
    close.type = "button";
    close.setAttribute("aria-label", "Dismiss agent message");
    close.textContent = "×";
    const to = document.createElement("div");
    to.className = "agent-meta agent-to";
    to.textContent = "to you";
    to.hidden = !answer.toYou;
    const body = this.renderBody(answer);
    const unavailable = document.createElement("div");
    unavailable.className = "agent-unavailable";
    unavailable.textContent = "Target unavailable";
    unavailable.hidden = true;
    const header = document.createElement("header");
    header.className = "agent-header";
    header.append(to, close);
    node.append(header, body, unavailable);
    const options = document.createElement("div");
    options.className = "agent-choices";
    for (const choice of answer.choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = choice;
      const ref = crypto.randomUUID();
      button.addEventListener("click", () => {
        for (const option of options.querySelectorAll("button")) option.disabled = true;
        void this.pick({
          ref,
          page: location.href,
          selector: `${AGENT_TARGET_PREFIX}${answer.id}`,
          selectedText: null,
          comment: choice,
          choice
        }).then((outcome) => {
          if ("commented" in outcome) {
            this.remove(answer.id);
            this.ui.showToast(`You chose: ${choice}`);
          } else if ("refused" in outcome && outcome.refused.choiceRefused === true) {
            this.remove(answer.id);
          } else {
            this.ui.showToast("refused" in outcome ? outcome.refused.error : "Choice not sent; try again shortly");
            for (const option of options.querySelectorAll("button")) option.disabled = false;
          }
        });
      });
      options.append(button);
    }
    node.append(options);
    const entry = this.placeTarget(answer);
    const selector = answer.anchor;
    const localAnchor = () => entry === void 0 ? selector !== void 0 && samePage(answer.page) : entry.here();
    const target = () => {
      if (entry !== void 0) return entry.target();
      if (!localAnchor() || selector === void 0) return void 0;
      const element = this.findTarget(selector);
      if (!element || element === this.ui.host) return void 0;
      if (selector.startsWith(AGENT_TARGET_PREFIX) && this.node(selector.slice(AGENT_TARGET_PREFIX.length)) === element) return element;
      if (element.getClientRects().length === 0) return void 0;
      return element;
    };
    const activate = () => {
      if (!localAnchor()) return;
      const found = target();
      if (found === void 0) this.ui.showToast("Element is unavailable");
      else this.revealElement(found);
    };
    const show = entry !== void 0 ? this.showLink(answer.id, entry) : selector === void 0 ? null : document.createElement("a");
    if (show !== null) {
      if (entry === void 0) {
        show.className = "agent-show";
        show.textContent = "Show";
        show.href = location.href;
        show.addEventListener("click", (event) => {
          event.preventDefault();
          activate();
        });
      }
      node.append(show);
    }
    node.addEventListener("click", (event) => {
      const interactive = event.target instanceof Element ? event.target.closest("a, button") : null;
      const selection = window.getSelection();
      if (!interactive && (!selection || selection.isCollapsed)) activate();
    });
    node.addEventListener("keydown", (event) => {
      if (event.target !== node || !["Enter", " "].includes(event.key) || !localAnchor()) return;
      event.preventDefault();
      activate();
    });
    close.addEventListener("click", () => {
      this.dismissed.add(answer.id);
      for (const id of this.dismissed) {
        if (this.dismissed.size <= MAX_ANSWERS) break;
        this.dismissed.delete(id);
      }
      this.persistDismissed();
      this.remove(answer.id);
    });
    const wrapper = document.createElement("div");
    wrapper.className = "agent-placement";
    const marker = document.createElement("div");
    marker.className = "agent-anchor-marker";
    marker.setAttribute("aria-hidden", "true");
    const link = makeLink();
    link.dataset.answerId = answer.id;
    const links = document.createElementNS(SVG_NS, "svg");
    links.classList.add("agent-anchor-layer");
    links.setAttribute("aria-hidden", "true");
    links.append(link);
    wrapper.append(links, marker);
    const highlight = (active) => {
      link.classList.toggle("active", active);
      marker.classList.toggle("active", active);
    };
    let hovered = false;
    node.addEventListener("mouseenter", () => {
      hovered = true;
      highlight(true);
    });
    node.addEventListener("mouseleave", () => {
      hovered = false;
      highlight(node.contains(this.ui.shadow.activeElement));
    });
    node.addEventListener("focusin", () => {
      highlight(true);
    });
    node.addEventListener("focusout", (event) => {
      highlight(hovered || event.relatedTarget instanceof Node && node.contains(event.relatedTarget));
    });
    const heights = /* @__PURE__ */ new Map();
    let measuredScale = this.ui.textScale;
    const heightAt = (width) => {
      if (measuredScale !== this.ui.textScale) {
        heights.clear();
        measuredScale = this.ui.textScale;
      }
      const cached = heights.get(width);
      if (cached !== void 0) return cached;
      const measure = node.cloneNode(true);
      measure.removeAttribute("data-message-id");
      measure.inert = true;
      measure.querySelector(".agent-timeout")?.remove();
      for (const detail of measure.querySelectorAll(".agent-show, .agent-unavailable")) {
        detail.hidden = true;
      }
      Object.assign(measure.style, {
        left: "-10000px",
        top: "0",
        width: `${width}px`,
        maxHeight: "none",
        visibility: "hidden"
      });
      this.ui.agentAnchored.append(measure);
      const height = measure.getBoundingClientRect().height;
      measure.remove();
      heights.set(width, height);
      return height;
    };
    const shown = {
      node,
      wrapper,
      marker,
      link,
      unavailable,
      show,
      target,
      localAnchor,
      hasPlace: entry !== void 0,
      heightAt,
      dropped: void 0,
      pendingDrop: void 0,
      placed: void 0,
      scroll: void 0,
      endMove: moveBy(header, node, (point, width, applied) => {
        shown.pendingDrop = { point, width: shown.dropped?.width ?? width / this.ui.textScale, applied };
        this.schedulePlacement();
      }, () => {
        const previous = shown.dropped;
        const pending = shown.pendingDrop;
        const placed = shown.placed;
        return () => {
          shown.placed = placed;
          shown.pendingDrop = pending;
          shown.dropped = previous;
          this.schedulePlacement();
        };
      }, () => {
        const rect = shown.placed?.target.getBoundingClientRect();
        return rect === void 0 ? void 0 : { x: rect.left, y: rect.top };
      }, (point, width, height, origin) => this.fitMoved(point, width, height, shown.scroll?.mode, origin?.y))
    };
    this.shown.set(answer.id, shown);
    this.schedulePlacement();
  }
  /** The share resolved this question; no session dismissal is needed. */
  remove(id) {
    if (this.recent.get(id)?.choices.length) this.recent.delete(id);
    const shown = this.shown.get(id);
    shown?.stopTimer?.();
    shown?.endMove();
    shown?.node.remove();
    shown?.wrapper.remove();
    this.shown.delete(id);
    this.prunePlaces();
    this.schedulePlacement();
  }
  /** Uses the same dismissal path as the close buttons. */
  clear() {
    for (const { node } of this.shown.values()) {
      node.querySelector(".agent-dismiss")?.click();
    }
  }
  /** Reposition cards and their connectors after the overlay changes size. */
  textSizeChanged() {
    this.schedulePlacement();
  }
  /** A changed duration gives every visible answer a fresh countdown. */
  setTimeoutSeconds(seconds) {
    if (seconds === this.timeoutSeconds) return;
    this.timeoutSeconds = seconds;
    for (const shown of this.shown.values()) this.startTimer(shown);
  }
  startTimer(shown) {
    shown.stopTimer?.();
    if (this.timeoutSeconds === 0 || !shown.node.isConnected) return;
    const { node } = shown;
    const bar = document.createElement("div");
    bar.className = "agent-timeout";
    bar.setAttribute("aria-hidden", "true");
    bar.style.animationDuration = `${this.timeoutSeconds}s`;
    bar.style.setProperty("--answer-timeout-steps", String(this.timeoutSeconds));
    node.append(bar);
    let stopped = false;
    let remaining = this.timeoutSeconds * 1e3;
    let started;
    let timer;
    let hovering = node.matches(":hover");
    let visible = false;
    const update = () => {
      if (stopped) return;
      const paused = !visible || document.hidden || hovering || node.contains(this.ui.shadow.activeElement);
      if (paused && started !== void 0) {
        remaining = Math.max(0, remaining - (performance.now() - started));
        started = void 0;
        clearTimeout(timer);
        timer = void 0;
      } else if (!paused && started === void 0) {
        started = performance.now();
        timer = window.setTimeout(() => {
          node.querySelector(".agent-dismiss")?.click();
        }, remaining);
      }
      bar.style.animationPlayState = paused ? "paused" : "running";
    };
    const enter = () => {
      hovering = true;
      update();
    };
    const leave = () => {
      hovering = false;
      update();
    };
    const focus = () => {
      queueMicrotask(update);
    };
    const visibility = new IntersectionObserver(([intersection]) => {
      visible = intersection?.isIntersecting ?? false;
      update();
    });
    visibility.observe(node);
    document.addEventListener("visibilitychange", update);
    node.addEventListener("mouseenter", enter);
    node.addEventListener("mouseleave", leave);
    node.addEventListener("focusin", focus);
    node.addEventListener("focusout", focus);
    shown.stopTimer = () => {
      stopped = true;
      clearTimeout(timer);
      visibility.disconnect();
      document.removeEventListener("visibilitychange", update);
      node.removeEventListener("mouseenter", enter);
      node.removeEventListener("mouseleave", leave);
      node.removeEventListener("focusin", focus);
      node.removeEventListener("focusout", focus);
      bar.remove();
      delete shown.stopTimer;
    };
    update();
  }
  persistDismissed() {
    try {
      sessionStorage.setItem(DISMISSED_KEY, JSON.stringify(Array.from(this.dismissed)));
    } catch {
    }
  }
  findTarget(selector) {
    try {
      return this.targetFor(selector);
    } catch {
      return null;
    }
  }
  schedulePlacement() {
    if (this.placementFrame !== void 0) return;
    this.placementFrame = requestAnimationFrame(() => {
      this.place();
    });
  }
  outlineFollows(source) {
    const outline = this.outlined;
    return outline !== void 0 && (outline.sources.has(source) || outline.mode === "document" && source === document);
  }
  onScroll = (event) => {
    const source = event.target;
    if (source === null) return;
    if (!this.outlineFollows(source) && !Array.from(this.shown.values()).some(({ scroll }) => scroll?.sources.has(source))) return;
    this.scrolled.add(source);
    if (this.scrollFrame !== void 0) return;
    this.scrollFrame = requestAnimationFrame(() => {
      this.scrollFrame = void 0;
      const sources = Array.from(this.scrolled);
      this.scrolled.clear();
      const measured = /* @__PURE__ */ new Map();
      const updates = /* @__PURE__ */ new Map();
      for (const { scroll, wrapper } of this.shown.values()) {
        if (scroll === void 0 || !sources.some((item) => scroll.sources.has(item))) continue;
        let rect = measured.get(scroll.target);
        if (rect === void 0) {
          rect = scroll.target.getBoundingClientRect();
          measured.set(scroll.target, rect);
        }
        updates.set(wrapper, `translate(${rect.left - scroll.rect.left}px, ${rect.top - scroll.rect.top}px)`);
      }
      const target = this.outlined?.target;
      const outline = target?.isConnected && sources.some((item) => this.outlineFollows(item)) ? measured.get(target) ?? target.getBoundingClientRect() : void 0;
      for (const [wrapper, transform] of updates) wrapper.style.setProperty("transform", transform);
      if (outline !== void 0) this.drawOutline(outline);
    });
  };
  place() {
    this.placementFrame = void 0;
    const { agentAnchored, agentGeneral } = this.ui;
    const anchored = /* @__PURE__ */ new Map();
    const cards = /* @__PURE__ */ new Map();
    let hovered;
    const fixed = /* @__PURE__ */ new Map();
    for (const entry of this.shown.values()) {
      const local = entry.localAnchor();
      const target = entry.target();
      cards.set(entry.node, entry);
      entry.node.classList.toggle("has-anchor", local);
      if (local && entry.node.tabIndex !== 0) {
        entry.node.tabIndex = 0;
        entry.node.setAttribute("aria-label", "Agent message; activate to show its anchor");
      } else if (!local && entry.node.hasAttribute("tabindex")) {
        entry.node.removeAttribute("tabindex");
        entry.node.removeAttribute("aria-label");
      }
      if (target !== void 0) anchored.set(entry, target);
      if (target === void 0 && entry.pendingDrop !== void 0) {
        const { point, width } = entry.pendingDrop;
        entry.dropped = { point, width, relative: false };
        entry.pendingDrop = void 0;
      }
      if (target === void 0 && entry.dropped?.relative === false) {
        fixed.set(entry, this.positionMoved(entry.node, entry.dropped));
      } else if (entry.dropped === void 0 && entry.node.classList.contains("manually-placed")) {
        entry.node.classList.remove("manually-placed");
        for (const property of ["left", "top", "width", "max-width", "max-height"]) {
          entry.node.style.removeProperty(property);
        }
      }
      if (target === void 0) this.placeGeneral(entry, local);
    }
    const preview = this.historyPreview;
    if (preview !== void 0) {
      if (preview.node.getClientRects().length > 0 && (preview.node.matches(":hover") || preview.node.contains(this.ui.shadow.activeElement))) {
        hovered = this.places.get(preview.id)?.target();
      } else {
        this.historyPreview = void 0;
      }
    }
    if (this.pendingReveal !== null) {
      const target = this.places.get(this.pendingReveal)?.target();
      if (target !== void 0) {
        this.pendingReveal = null;
        this.revealElement(target);
      }
    }
    if (this.highlightTimer === void 0) {
      if (hovered !== void 0) this.outline(hovered);
      else {
        this.outlined = void 0;
        this.ui.messageHighlight.style.display = "none";
      }
    }
    if (this.pdf !== void 0 && this.sections === null) {
      this.sections = this.ui.shadow.querySelector("#pdf-sections");
      if (this.sections !== null) {
        this.geometry.observe(this.sections);
        this.obstacleChanges.observe(this.sections, { attributes: true });
      }
    }
    const sectionsHeight = this.sections?.offsetHeight ?? 0;
    const scale = this.ui.textScale;
    const sectionsOffset = `${sectionsHeight === 0 ? 0 : sectionsHeight + 14 * scale}px`;
    if (agentGeneral.style.getPropertyValue("--sections-offset") !== sectionsOffset) {
      agentGeneral.style.setProperty("--sections-offset", sectionsOffset);
    }
    const obstacles = [
      this.ui.bar,
      this.ui.list,
      this.ui.panel,
      this.ui.dialog,
      ...this.sections === null ? [] : [this.sections],
      ...this.pdf === void 0 ? [] : document.querySelectorAll("#pdf-feedback > :not(:empty)")
    ];
    const controls = new Map(obstacles.filter((node) => node.getClientRects().length > 0 && getComputedStyle(node).visibility === "visible").map((node) => [node, node.getBoundingClientRect()]));
    for (const [entry, box] of fixed) controls.set(entry.node, box);
    const occupied = Array.from(controls.values());
    const targets = new Set(anchored.values());
    for (const target of this.observedTargets) {
      if (targets.has(target)) continue;
      this.geometry.unobserve(target);
      this.visibility.unobserve(target);
      this.visibleTargets.delete(target);
    }
    for (const target of targets) {
      if (this.observedTargets.has(target)) continue;
      this.geometry.observe(target);
      this.visibility.observe(target);
    }
    this.observedTargets = targets;
    const view = answerViewport();
    const origin = agentAnchored.getBoundingClientRect();
    const pdfViewer = document.documentElement.hasAttribute("data-jamparty-pdf");
    const targetBoxes = /* @__PURE__ */ new Map();
    const pages = /* @__PURE__ */ new Map();
    for (const target of targets) {
      const parent = cards.get(target);
      if (parent !== void 0 && anchored.has(parent)) continue;
      targetBoxes.set(target, target.getBoundingClientRect());
      const page = pdfViewer ? target.closest(".pdf-page") : null;
      if (page !== null) pages.set(target, page.getBoundingClientRect());
    }
    const general = new Map(
      Array.from(agentGeneral.children).filter((node) => !node.classList.contains("manually-placed")).map((node) => [node, node.getBoundingClientRect()])
    );
    const placedCards = [];
    const pending = new Set(anchored.keys());
    const placements = /* @__PURE__ */ new Map();
    const solve = (entry) => {
      if (!pending.delete(entry)) return;
      const target = anchored.get(entry);
      if (target === void 0) return;
      const parent = cards.get(target);
      if (parent !== void 0) solve(parent);
      const rect = targetBoxes.get(target);
      const previous = general.get(entry.node);
      general.delete(entry.node);
      const page = pages.get(target);
      const inherited = parent === void 0 ? void 0 : placements.get(parent)?.scroll;
      const scroll = rect === void 0 ? void 0 : inherited ?? scrollRelationship(target, rect);
      const mode = scroll?.mode ?? "document";
      const viewport = mode !== "document";
      const frame = viewport ? { x: 0, y: 0 } : { x: origin.left, y: origin.top };
      const local = rect === void 0 ? void 0 : translate(rect, -frame.x, -frame.y);
      const localPage = page === void 0 ? void 0 : translate(page, -frame.x, -frame.y);
      const cached = entry.placed;
      const previousBox = cached === void 0 ? void 0 : translate(cached.placement.box, frame.x, frame.y);
      const move = entry.pendingDrop;
      if (rect !== void 0 && move !== void 0) {
        const { point, width } = move;
        entry.dropped = { point: { x: point.x - rect.left, y: point.y - rect.top }, width, relative: true };
        entry.pendingDrop = void 0;
      }
      const dropped = entry.dropped;
      const moved = rect === void 0 || dropped === void 0 ? void 0 : this.positionMoved(entry.node, dropped, rect, mode, move !== void 0);
      if (move !== void 0 && moved !== void 0 && rect !== void 0 && dropped !== void 0) {
        dropped.point = { x: moved.left - rect.left, y: moved.top - rect.top };
        move.applied?.(dropped.point);
      }
      const obstacles2 = new Map([...controls, ...general]);
      for (const [node, box] of targetBoxes) {
        if (node !== target) obstacles2.set(node, box);
      }
      const blocked = cached !== void 0 && previousBox !== void 0 && Array.from(obstacles2).some(([node, box]) => !sameBox(cached.obstacles.get(node), box) && intersects(box, previousBox));
      const height = view.bottom - view.top;
      const layoutView = rect === void 0 || rect.bottom > view.top && rect.top < view.bottom ? view : translate(view, 0, (rect.top + rect.bottom - view.top - view.bottom) / 2);
      const placement = rect === void 0 ? void 0 : moved !== void 0 ? { box: moved, side: nearestSide(rect, moved), maxHeight: height } : cached !== void 0 && cached.target === target && previousBox !== void 0 && sameBox(cached.rect, local) && sameBox(cached.page, localPage) && sameBox(cached.view, view) && cached.scale === scale && entry.scroll?.mode === mode && !blocked && !placedCards.some((box) => intersects(box, previousBox)) ? { ...cached.placement, box: previousBox } : placeAnswer(entry.heightAt, layoutView, rect, page, [
        ...occupied,
        ...general.values(),
        ...Array.from(targetBoxes.values()).filter((box) => box !== rect)
      ], blocked ? oppositeSide[cached.placement.side] : void 0, scale);
      if (placement === void 0 || rect === void 0 || scroll === void 0) {
        this.placeGeneral(entry, false);
        if (previous === void 0) {
          for (const node of general.keys()) {
            const box2 = node.getBoundingClientRect();
            general.set(node, box2);
            if (targetBoxes.has(node)) targetBoxes.set(node, box2);
          }
          this.schedulePlacement();
        }
        const box = previous ?? entry.node.getBoundingClientRect();
        general.set(entry.node, box);
        if (targets.has(entry.node)) targetBoxes.set(entry.node, box);
        return;
      }
      occupied.push(placement.box);
      placedCards.push(placement.box);
      entry.scroll = scroll;
      if (local !== void 0) entry.placed = {
        target,
        rect: local,
        page: localPage,
        view,
        obstacles: obstacles2,
        scale,
        placement: { ...placement, box: translate(placement.box, -frame.x, -frame.y) }
      };
      if (targets.has(entry.node)) targetBoxes.set(entry.node, placement.box);
      placements.set(entry, { rect, placement, scroll });
    };
    const ordered = Array.from(anchored).sort(([, a], [, b]) => (targetBoxes.get(a)?.top ?? Infinity) - (targetBoxes.get(b)?.top ?? Infinity));
    for (const [entry] of ordered) solve(entry);
    for (const [entry, { rect, placement, scroll }] of placements) {
      const viewport = scroll.mode !== "document";
      entry.wrapper.classList.toggle("viewport-anchored", viewport);
      const target = entry.placed?.target;
      entry.wrapper.classList.toggle("target-offscreen", target !== void 0 && !(this.visibleTargets.get(target) ?? (rect.bottom > 0 && rect.top < window.innerHeight && rect.right > 0 && rect.left < window.innerWidth)));
      entry.wrapper.style.transform = "";
      const x = viewport ? 0 : -origin.left;
      const y = viewport ? 0 : -origin.top;
      entry.unavailable.hidden = true;
      if (entry.show !== null) entry.show.hidden = true;
      if (entry.node.parentElement !== entry.wrapper) {
        if (entry.node.parentElement === agentGeneral) this.schedulePlacement();
        entry.wrapper.prepend(entry.node);
      }
      if (entry.wrapper.parentElement !== agentAnchored) agentAnchored.append(entry.wrapper);
      const box = translate(placement.box, x, y);
      entry.node.style.left = `${box.left}px`;
      entry.node.style.top = `${box.top}px`;
      entry.node.style.width = `${box.right - box.left}px`;
      entry.node.style.maxHeight = `${placement.maxHeight}px`;
      drawAnswerLink(entry.marker, entry.link, translate(rect, x, y), { ...placement, box });
    }
    agentGeneral.classList.toggle("scrollable", agentGeneral.scrollHeight > agentGeneral.clientHeight);
    for (const entry of this.shown.values()) {
      if (entry.stopTimer === void 0) this.startTimer(entry);
    }
  }
  fitMoved(point, width, height, mode, anchorTop) {
    if (mode !== "document" || anchorTop === void 0) return fitPosition(point, width, height);
    const view = answerViewport();
    const end = (document.scrollingElement?.scrollHeight ?? window.innerHeight) - window.innerHeight;
    return {
      x: clamp(point.x, view.left, view.right - width),
      y: clamp(
        point.y,
        Math.max(view.top - window.scrollY, anchorTop - (view.bottom - view.top)),
        Math.min(view.bottom + end - window.scrollY - height, anchorTop + (view.bottom - view.top) - height)
      )
    };
  }
  positionMoved(node, dropped, target, mode, moving = false) {
    const view = answerViewport();
    node.classList.add("manually-placed");
    const preferredWidth = dropped.width * this.ui.textScale;
    node.style.width = `${preferredWidth}px`;
    node.style.maxWidth = `${view.right - view.left}px`;
    node.style.maxHeight = `${view.bottom - view.top}px`;
    const { width, height } = node.isConnected ? node.getBoundingClientRect() : { width: Math.min(preferredWidth, view.right - view.left), height: node.scrollHeight };
    let point = dropped.point;
    if (target !== void 0 && dropped.relative) {
      point = { x: target.left + point.x, y: target.top + point.y };
    }
    if (mode !== "tracked" || moving) {
      point = this.fitMoved(point, width, height, mode, target?.top);
    } else {
      point.x = clamp(point.x, view.left, view.right - width);
    }
    if (target === void 0) {
      node.style.left = `${point.x}px`;
      node.style.top = `${point.y}px`;
    }
    return { left: point.x, top: point.y, right: point.x + width, bottom: point.y + height };
  }
  placeGeneral(entry, unavailable) {
    if (entry.unavailable.hidden === unavailable) entry.unavailable.hidden = !unavailable;
    if (entry.show !== null) {
      const hidden = unavailable || !entry.hasPlace && !entry.localAnchor();
      if (entry.show.hidden !== hidden) entry.show.hidden = hidden;
    }
    entry.placed = void 0;
    entry.scroll = void 0;
    if (entry.node.parentElement !== this.ui.agentGeneral) this.ui.agentGeneral.append(entry.node);
    entry.wrapper.remove();
    if (entry.dropped?.relative === false) return;
    entry.node.classList.remove("manually-placed");
    for (const property of ["left", "top", "width", "max-width", "max-height"]) {
      entry.node.style.removeProperty(property);
    }
  }
  outline(target) {
    const rect = target.getBoundingClientRect();
    this.outlined = scrollRelationship(target, rect);
    this.drawOutline(rect);
  }
  drawOutline(rect) {
    const { messageHighlight } = this.ui;
    messageHighlight.style.display = "block";
    messageHighlight.style.left = `${rect.left}px`;
    messageHighlight.style.top = `${rect.top}px`;
    messageHighlight.style.width = `${rect.width}px`;
    messageHighlight.style.height = `${rect.height}px`;
  }
  showHighlight(target) {
    this.outline(target);
    if (this.highlightTimer !== void 0) clearTimeout(this.highlightTimer);
    this.highlightTimer = window.setTimeout(() => {
      this.highlightTimer = void 0;
      this.schedulePlacement();
    }, 1800);
  }
  /** Scrolls to the element `selector` names and highlights it briefly. */
  reveal(selector) {
    const target = this.findTarget(selector);
    if (!target || target.getClientRects().length === 0) {
      this.ui.showToast("Element is unavailable");
      return;
    }
    this.revealElement(target);
  }
  revealElement(target) {
    target.scrollIntoView({ block: "center", inline: "nearest" });
    requestAnimationFrame(() => {
      if (target.isConnected) this.showHighlight(target);
    });
  }
  followElementLink(link, event) {
    const href = link.getAttribute("href") ?? "";
    if (!href.toLowerCase().startsWith("jamparty:")) return;
    event.preventDefault();
    let selector;
    try {
      selector = decodeURIComponent(href.slice("jamparty:".length));
    } catch {
      this.ui.showToast("Element link is invalid");
      return;
    }
    this.reveal(selector);
  }
};

// client/src/connection.ts
var Connection = class {
  constructor(changed) {
    this.changed = changed;
  }
  changed;
  socketOpen = false;
  link = null;
  /** The share is reachable through the joiner, status line or not. */
  reachable = false;
  get isReachable() {
    return this.reachable;
  }
  get view() {
    let colour = "green";
    let text = "connected";
    if (!this.socketOpen) {
      colour = "amber";
      text = "reconnecting to jamparty join…";
    } else if (this.link?.connected !== true) {
      colour = "grey";
      text = "share unreachable";
    } else if (this.link.status !== null) {
      colour = "grey";
      text = this.link.status;
    }
    if (colour === "green" && this.link?.path) text += ` · ${this.link.path}`;
    return { colour, text };
  }
  setOpen(open) {
    this.socketOpen = open;
    this.render();
  }
  setLink(link) {
    this.link = link;
    this.render();
  }
  render() {
    const reachable = this.socketOpen && this.link?.connected === true;
    const reached = reachable && !this.reachable;
    this.reachable = reachable;
    this.changed(this.view, reached);
  }
};

// client/src/remembered.ts
var MAX_AGE_SECS = 365 * 24 * 60 * 60;
function isSwitch(value) {
  return value === "on" || value === "off";
}
function isTheme(value) {
  return value === "auto" || value === "dark" || value === "light";
}
function isTextSize(value) {
  return value === "small" || value === "default" || value === "large" || value === "larger";
}
function isCorner(value) {
  return value === "bottom-right" || value === "bottom-left" || value === "top-right" || value === "top-left";
}
function parse(value) {
  if (!isRecord(value)) return {};
  const change = {};
  const { name, keys, commentMode, showCursor, corner, theme, textSize, answerTimeout } = value;
  if (typeof name === "string") change.name = name;
  if (isRecord(keys) && typeof keys.commentMode === "string" && typeof keys.showCursor === "string" && typeof keys.clearAnswers === "string") {
    change.keys = {
      commentMode: keys.commentMode,
      showCursor: keys.showCursor,
      clearAnswers: keys.clearAnswers
    };
  }
  if (isSwitch(commentMode)) change.commentMode = commentMode;
  if (isSwitch(showCursor)) change.showCursor = showCursor;
  if (typeof answerTimeout === "number" && Number.isInteger(answerTimeout) && answerTimeout >= 0 && answerTimeout <= MAX_ANSWER_TIMEOUT) change.answerTimeout = answerTimeout;
  if (isTheme(theme)) change.theme = theme;
  if (isTextSize(textSize)) change.textSize = textSize;
  if (isCorner(corner)) change.corner = corner;
  return change;
}
function remembered() {
  const prefix = `${SETTINGS_COOKIE}=`;
  const cookie = document.cookie.split("; ").find((part) => part.startsWith(prefix));
  if (cookie === void 0) return {};
  try {
    return parse(JSON.parse(decodeURIComponent(cookie.slice(prefix.length))));
  } catch {
    return {};
  }
}
function keep(change) {
  const value = encodeURIComponent(JSON.stringify(change));
  document.cookie = `${SETTINGS_COOKIE}=${value}; path=/; max-age=${MAX_AGE_SECS}; samesite=strict`;
}
function remember(change) {
  keep({ ...remembered(), ...parse(change) });
}
function forget() {
  document.cookie = `${SETTINGS_COOKIE}=; path=/; max-age=0; samesite=strict`;
}
function missing(change, settings) {
  const needed = {};
  let any = false;
  if (change.name !== void 0 && (change.name !== settings.name || settings.askName)) {
    needed.name = change.name;
    any = true;
  }
  if (change.keys !== void 0 && (change.keys.commentMode !== settings.keys.commentMode || change.keys.showCursor !== settings.keys.showCursor || change.keys.clearAnswers !== settings.keys.clearAnswers)) {
    needed.keys = change.keys;
    any = true;
  }
  if (change.commentMode !== void 0 && change.commentMode !== settings.commentMode) {
    needed.commentMode = change.commentMode;
    any = true;
  }
  if (change.showCursor !== void 0 && change.showCursor !== settings.showCursor) {
    needed.showCursor = change.showCursor;
    any = true;
  }
  if (change.answerTimeout !== void 0 && change.answerTimeout !== settings.answerTimeout) {
    needed.answerTimeout = change.answerTimeout;
    any = true;
  }
  if (change.corner !== void 0 && change.corner !== settings.corner) {
    needed.corner = change.corner;
    any = true;
  }
  if (change.theme !== void 0 && change.theme !== settings.theme) {
    needed.theme = change.theme;
    any = true;
  }
  if (change.textSize !== void 0 && change.textSize !== settings.textSize) {
    needed.textSize = change.textSize;
    any = true;
  }
  return any ? needed : null;
}

// client/src/panel.ts
var CORNERS = ["bottom-right", "bottom-left", "top-right", "top-left"];
function isCorner2(value) {
  return CORNERS.some((corner) => corner === value);
}
var ACTION_NAMES = {
  commentMode: "comment mode",
  showCursor: "show my cursor",
  clearAnswers: "clear answers"
};
function normalizeKey(key) {
  return key.length === 1 ? key.toLowerCase() : key;
}
function keyLabel(key) {
  return key.length === 1 ? key.toUpperCase() : key;
}
function isEditing(event) {
  return event.composedPath().some(
    (node) => node instanceof HTMLElement && (node.isContentEditable || node.matches("input, textarea, select"))
  );
}
function initials(name) {
  const parts = name.split("@")[0]?.split(/[._+-]+/u).filter((part) => part !== "") ?? [];
  const letters = parts.slice(0, 2).map((part) => part.charAt(0).toUpperCase()).join("");
  return letters === "" ? "?" : letters;
}
function failure(outcome) {
  return outcome === "saved" ? null : outcome.refused.error;
}
function colourOf(person) {
  return PALETTE[person.colour % PALETTE.length] ?? "#555";
}
var Panel = class {
  constructor(ui, changeSettings) {
    this.ui = ui;
    this.changeSettings = changeSettings;
    ui.clearAnswers.addEventListener("click", () => {
      this.onClearAnswers();
    });
    ui.answerTimeout.addEventListener("change", () => {
      if (ui.answerTimeout.reportValidity()) {
        void this.change({ answerTimeout: ui.answerTimeout.valueAsNumber });
      }
    });
    ui.gear.addEventListener("click", () => {
      this.toggle();
    });
    ui.commentMode.addEventListener("click", () => {
      this.flip("commentMode");
    });
    ui.showCursor.addEventListener("click", () => {
      this.flip("showCursor");
    });
    this.themes = ui.shadow.querySelectorAll("button[data-theme]");
    for (const button of this.themes) {
      button.addEventListener("click", () => {
        const theme = button.dataset.theme;
        if (isTheme(theme) && this.settings?.theme !== theme) void this.change({ theme });
      });
    }
    this.textSizes = ui.shadow.querySelectorAll("button[data-text-size]");
    for (const button of this.textSizes) {
      button.addEventListener("click", () => {
        const textSize = button.dataset.textSize;
        if (isTextSize(textSize) && this.settings?.textSize !== textSize) {
          void this.change({ textSize });
        }
      });
    }
    for (const button of ui.corners) {
      button.addEventListener("click", () => {
        const corner = button.dataset.corner;
        if (!isCorner2(corner) || this.settings?.corner === corner) return;
        this.message("");
        void this.change({ corner });
      });
    }
    for (const [action, chip] of this.chips()) {
      chip.addEventListener("click", () => {
        this.listen(action);
      });
    }
    ui.nameField.addEventListener("keydown", (event) => {
      if (event.key === "Enter") {
        event.preventDefault();
        this.saveName();
      }
    });
    ui.nameField.addEventListener("change", () => {
      this.saveName();
    });
    document.addEventListener("keydown", (event) => {
      this.onKey(event);
    }, true);
    document.addEventListener("pointerdown", (event) => {
      const path = event.composedPath();
      if (this.isOpen && !path.includes(ui.panel) && !path.includes(ui.gear)) this.close();
    }, true);
  }
  ui;
  changeSettings;
  themes;
  textSizes;
  settings = null;
  listening = null;
  /** Called by the clear button or its binding. */
  onClearAnswers = () => void 0;
  /** Called when the panel opens (the comment list closes then). */
  onOpen = () => void 0;
  /** Called with the settings whenever they change. */
  onSettings = () => void 0;
  chips() {
    return [["commentMode", this.ui.commentModeKey], ["showCursor", this.ui.showCursorKey], ["clearAnswers", this.ui.clearAnswersKey]];
  }
  get isOpen() {
    return !this.ui.panel.hidden;
  }
  get commentMode() {
    return this.settings?.commentMode !== "off";
  }
  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }
  open() {
    this.onOpen();
    this.ui.panel.hidden = false;
    this.ui.gear.setAttribute("aria-expanded", "true");
  }
  close() {
    this.stopListening();
    this.ui.panel.hidden = true;
    this.ui.gear.setAttribute("aria-expanded", "false");
    this.ui.panelMessage.textContent = "";
  }
  message(text) {
    this.ui.panelMessage.textContent = text;
  }
  /** The settings from the joiner (its hello or a settings message). */
  show(settings) {
    this.settings = settings;
    const { ui } = this;
    ui.commentMode.setAttribute("aria-checked", String(settings.commentMode === "on"));
    ui.answerTimeout.value = String(settings.answerTimeout);
    ui.showCursor.setAttribute("aria-checked", String(settings.showCursor === "on"));
    for (const [action, chip] of this.chips()) {
      if (this.listening !== action) chip.textContent = keyLabel(settings.keys[action]);
    }
    if (ui.shadow.activeElement !== ui.nameField) ui.nameField.value = settings.name;
    for (const button of ui.corners) {
      button.setAttribute("aria-pressed", String(button.dataset.corner === settings.corner));
    }
    ui.host.dataset.corner = settings.corner;
    ui.host.dataset.theme = settings.theme;
    for (const button of this.themes) {
      button.setAttribute("aria-pressed", String(button.dataset.theme === settings.theme));
    }
    const textSize = isTextSize(settings.textSize) ? settings.textSize : "default";
    ui.setTextSize(textSize);
    for (const button of this.textSizes) {
      button.setAttribute("aria-pressed", String(button.dataset.textSize === textSize));
    }
    this.onSettings(settings);
  }
  /**
   * Asks the joiner for a change. What it took comes as a settings
   * message, in order with everyone else's; only a failure is shown here.
   */
  async change(change) {
    const failed = failure(await this.changeSettings(change));
    if (failed !== null) this.message(failed);
  }
  flip(action) {
    const settings = this.settings;
    if (settings === null) return;
    const value = settings[action] === "on" ? "off" : "on";
    this.message("");
    void this.change(action === "commentMode" ? { commentMode: value } : { showCursor: value });
  }
  saveName() {
    const name = this.ui.nameField.value.trim();
    if (this.settings === null || name === this.settings.name) return;
    this.message("");
    void this.change({ name });
  }
  listen(action) {
    this.stopListening();
    this.listening = action;
    const chip = this.chips().find(([name]) => name === action)?.[1];
    if (chip === void 0) return;
    chip.dataset.listening = "true";
    chip.textContent = "press a key…";
    this.message("");
  }
  stopListening() {
    this.listening = null;
    for (const [action, chip] of this.chips()) {
      chip.dataset.listening = "false";
      if (this.settings !== null) chip.textContent = keyLabel(this.settings.keys[action]);
    }
  }
  /** The next key press while a chip listens becomes its binding. */
  rebind(action, event) {
    event.preventDefault();
    event.stopImmediatePropagation();
    const settings = this.settings;
    if (event.key === "Escape" || settings === null) {
      this.stopListening();
      return;
    }
    if (MODIFIER_KEYS.includes(event.key)) return;
    const key = normalizeKey(event.key);
    const other = this.chips().map(([name]) => name).find(
      (name) => name !== action && normalizeKey(settings.keys[name]) === key
    );
    if (RESERVED_KEYS.includes(key)) {
      this.stopListening();
      this.message(`${keyLabel(key)} is used by jamparty`);
      return;
    }
    if (PAGE_KEYS.includes(key)) {
      this.stopListening();
      this.message(`${key === " " ? "Space" : key} is the page's`);
      return;
    }
    if (other !== void 0) {
      this.stopListening();
      this.message(`${keyLabel(key)} is taken by ${ACTION_NAMES[other]}`);
      return;
    }
    const keys = { ...settings.keys, [action]: key };
    this.stopListening();
    void this.change({ keys });
  }
  onKey(event) {
    if (this.listening !== null) {
      this.rebind(this.listening, event);
      return;
    }
    const settings = this.settings;
    if (settings === null || event.defaultPrevented || event.isComposing || event.metaKey || event.ctrlKey || event.altKey || isEditing(event)) return;
    const key = normalizeKey(event.key);
    for (const action of ["commentMode", "showCursor", "clearAnswers"]) {
      if (normalizeKey(settings.keys[action]) === key) {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (!event.repeat) {
          if (action === "clearAnswers") this.onClearAnswers();
          else this.flip(action);
        }
        return;
      }
    }
  }
  /** Who is here: the count and an avatar each, the joiner's own outlined. */
  showPeople(here, you) {
    const { hereCount, avatars } = this.ui;
    hereCount.textContent = `${here.length} here`;
    const ordered = [
      ...here.filter((person) => person.key === you),
      ...here.filter((person) => person.key !== you)
    ];
    avatars.replaceChildren(...ordered.map((person) => {
      const item = document.createElement("li");
      item.className = "avatar";
      const mine = person.key === you;
      item.dataset.you = String(mine);
      item.dataset.key = person.key;
      item.title = mine ? `${person.name} (you)` : person.name;
      item.textContent = initials(person.name);
      if (!mine) item.style.background = colourOf(person);
      return item;
    }));
  }
};

// client/src/cursors.ts
var INTERVAL_MS = 1e3 / CURSOR_RATE;
var CURSOR_GONE_MS = CURSOR_FADE_MS + 1e3;
function fraction(offset, size) {
  if (size <= 0) return 0;
  return Math.round(Math.min(1, Math.max(0, offset / size)) * FRACTION_ONE);
}
var CursorSender = class {
  constructor(ui, send) {
    this.ui = ui;
    this.send = send;
    document.addEventListener("pointermove", (event) => {
      this.moved(event);
    }, {
      capture: true,
      passive: true
    });
    document.documentElement.addEventListener("pointerleave", () => {
      this.left();
    });
    window.addEventListener("blur", () => {
      this.left();
    });
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") this.left();
    });
  }
  ui;
  send;
  on = false;
  /**
   * The socket is up and has been told this page's URL: pointers go out
   * on it, after that report, so the joiner reads them on that page.
   */
  placed = false;
  /** The latest pointer position not yet sent. */
  latest = null;
  lastSent = 0;
  timer;
  /** Whether others currently see this pointer (a position went out). */
  present = false;
  get active() {
    return this.on && this.placed;
  }
  /** Forgets everything not sent yet. */
  reset() {
    if (this.timer !== void 0) clearTimeout(this.timer);
    this.timer = void 0;
    this.latest = null;
    this.present = false;
  }
  /** The cursor switch; nothing at all is sent while it is off. */
  setOn(on) {
    this.on = on;
    if (!on) this.reset();
  }
  /**
   * Whether the joiner has this page's URL: true right after telling it,
   * false while the socket is down or the page cannot say where it is.
   */
  setPlaced(placed) {
    this.placed = placed;
    if (!placed) this.reset();
  }
  moved(event) {
    if (!this.active || event.pointerType === "touch") return;
    this.latest = { x: event.clientX, y: event.clientY };
    this.schedule();
  }
  /** Sends the latest movement now, or at the end of the rate window. */
  schedule() {
    if (this.timer !== void 0) return;
    const wait = this.lastSent + INTERVAL_MS - performance.now();
    if (wait <= 0) {
      this.flush();
    } else {
      this.timer = window.setTimeout(() => {
        this.timer = void 0;
        this.flush();
      }, wait);
    }
  }
  flush() {
    const latest = this.latest;
    this.latest = null;
    if (latest === null || !this.active) return;
    const position = this.position(latest.x, latest.y);
    if (position === null) {
      this.left();
      return;
    }
    this.lastSent = performance.now();
    this.present = this.send(position);
  }
  /** The pointer left the page: one message without a position. */
  left() {
    if (this.timer !== void 0) clearTimeout(this.timer);
    this.timer = void 0;
    this.latest = null;
    if (!this.active || !this.present) return;
    this.present = false;
    this.send(null);
  }
  /** The element under the point (not jamparty's own overlay) and the offset in its box. */
  position(x, y) {
    const element = document.elementsFromPoint(x, y).find((candidate) => candidate !== this.ui.host && !this.ui.host.contains(candidate));
    if (element === void 0) return null;
    const pdf = document.documentElement.hasAttribute("data-jamparty-pdf");
    const target = pdf ? element.closest(".pdf-page") : element;
    if (target === null) return null;
    const selector = cssSelector(target);
    if (selector.length > MAX_CURSOR_SELECTOR) return null;
    const rect = target.getBoundingClientRect();
    return {
      selector,
      x: fraction(x - rect.left, rect.width),
      y: fraction(y - rect.top, rect.height)
    };
  }
};
var SVG_NS2 = "http://www.w3.org/2000/svg";
var CursorLayer = class {
  constructor(ui) {
    this.ui = ui;
  }
  ui;
  drawn = /* @__PURE__ */ new Map();
  people = /* @__PURE__ */ new Map();
  /** This page's URL, as last said. */
  url = null;
  frame;
  /**
   * Whether `key`'s pointer may stay drawn: here, on this page, cursor on.
   * Presence may lag a pointer by a moment (the share sends it in
   * batches), so a pointer is drawn for anyone here, and taken away when
   * presence says they left the page or switched off.
   */
  shows(key) {
    const person = this.people.get(key);
    return person?.cursor === "on" && this.url !== null && person.url === this.url;
  }
  /** Someone's pointer on this page moved, or (null) left it. */
  update(key, position) {
    if (position === null || !this.people.has(key)) {
      this.remove(key);
      return;
    }
    if (!this.drawn.has(key) && this.drawn.size >= MAX_JOINERS) return;
    let element;
    try {
      element = document.querySelector(position.selector);
    } catch {
      element = null;
    }
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
  setPeople(here, url) {
    this.people = new Map(here.map((person) => [person.key, person]));
    this.url = url;
    for (const key of [...this.drawn.keys()]) {
      if (!this.shows(key)) this.remove(key);
    }
    for (const drawn of this.drawn.values()) this.paint(drawn);
  }
  clear() {
    for (const key of [...this.drawn.keys()]) this.remove(key);
  }
  remove(key) {
    this.drawn.get(key)?.node.remove();
    this.drawn.delete(key);
  }
  create(key) {
    const node = document.createElement("div");
    node.className = "cursor";
    node.dataset.key = key;
    const svg = document.createElementNS(SVG_NS2, "svg");
    svg.setAttribute("width", "16");
    svg.setAttribute("height", "18");
    svg.setAttribute("viewBox", "0 0 16 18");
    const arrow = document.createElementNS(SVG_NS2, "path");
    arrow.setAttribute("d", "M1.5 1.5 L1.5 14.5 L5.2 11.2 L7.8 16.5 L10.2 15.4 L7.7 10.3 L12.8 10.3 Z");
    arrow.setAttribute("stroke", "#ffffff");
    arrow.setAttribute("stroke-width", "1.4");
    arrow.setAttribute("stroke-linejoin", "round");
    svg.append(arrow);
    const tag = document.createElement("span");
    tag.className = "tag";
    node.append(svg, tag);
    this.ui.cursors.append(node);
    const drawn = {
      node,
      tag,
      arrow,
      element: document.documentElement,
      x: 0,
      y: 0,
      shownX: null,
      shownY: null,
      updated: performance.now()
    };
    this.drawn.set(key, drawn);
    return drawn;
  }
  paint(drawn) {
    const person = this.people.get(drawn.node.dataset.key ?? "");
    const colour = person === void 0 ? "#555" : colourOf(person);
    drawn.arrow.setAttribute("fill", colour);
    drawn.tag.style.background = colour;
    drawn.tag.textContent = person?.name ?? "";
  }
  schedule() {
    if (this.frame !== void 0) return;
    this.frame = requestAnimationFrame(() => {
      this.animate();
    });
  }
  /**
   * Glides each cursor towards its point (kept on its element as the page
   * scrolls), fades idle ones and removes them once faded.
   */
  animate() {
    this.frame = void 0;
    const now = performance.now();
    for (const [key, drawn] of [...this.drawn]) {
      if (now - drawn.updated > CURSOR_GONE_MS) {
        this.remove(key);
        continue;
      }
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
      drawn.node.dataset.faded = String(now - drawn.updated > CURSOR_FADE_MS);
    }
    if (this.drawn.size > 0) this.schedule();
  }
};

// client/src/location.ts
function pageUrl() {
  const url = new URL(window.location.href);
  const raw = `${url.pathname}${url.search}${url.hash}`;
  const encoded = raw.replace(/[^\x21-\x7e]|["\\]/gu, (character) => encodeURIComponent(character));
  return encoded.startsWith("/") && encoded.length <= MAX_PAGE_PATH ? encoded : null;
}
function isThisPage(page) {
  try {
    const url = new URL(page);
    const here = window.location;
    return url.pathname === here.pathname && url.search === here.search && url.hash === here.hash;
  } catch {
    return false;
  }
}
function watchUrl(listener) {
  let last = pageUrl();
  const check = () => {
    const now = pageUrl();
    if (now === last) return;
    last = now;
    listener(now);
  };
  for (const method of ["pushState", "replaceState"]) {
    const original = history[method].bind(history);
    history[method] = (...args) => {
      original(...args);
      check();
    };
  }
  window.addEventListener("popstate", check);
  window.addEventListener("hashchange", check);
}

// client/src/draft.ts
function parseKind(value) {
  switch (value) {
    case "element":
    case "page":
    case "answer":
    case "pdf":
      return value;
    default:
      return null;
  }
}
function parseTarget(value) {
  if (!isRecord(value)) return null;
  const kind = parseKind(value.kind);
  const { selector } = value;
  if (kind === null || typeof selector !== "string") return null;
  return { kind, selector };
}
function parse2(value) {
  if (!isRecord(value)) return null;
  const { page, unavailable, selectedText, comment } = value;
  const target = parseTarget(value.target);
  if (typeof page !== "string" || typeof comment !== "string" || target === null) return null;
  const pdf = parsePdfAnchor(value.pdf);
  return {
    page,
    ...pdf === void 0 ? {} : { pdf },
    target,
    unavailable: unavailable === true,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment,
    ...typeof value.editRef === "string" ? { editRef: value.editRef } : {},
    ...Array.isArray(value.originalFiles) && value.originalFiles.every((index) => typeof index === "number" && Number.isInteger(index) && index >= 0) ? { originalFiles: value.originalFiles } : {},
    fileCount: typeof value.fileCount === "number" && value.fileCount > 0 ? value.fileCount : 0
  };
}
function saveDraft(draft) {
  try {
    sessionStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
  } catch {
  }
}
function clearDraft() {
  try {
    sessionStorage.removeItem(DRAFT_KEY);
  } catch {
  }
}
function loadDraft() {
  try {
    const draft = parse2(JSON.parse(sessionStorage.getItem(DRAFT_KEY) ?? "null"));
    return draft !== null && isThisPage(draft.page) ? draft : null;
  } catch {
    return null;
  }
}

// client/src/dialog.ts
var ELEMENT_LABEL = "Comment:";
var ANSWER_LABEL = "Comment on agent response:";
var GONE = "The element is gone; this will be a page comment.";
function labelFor(kind) {
  switch (kind) {
    case "element":
    case "page":
    case "pdf":
      return ELEMENT_LABEL;
    case "answer":
      return ANSWER_LABEL;
  }
}
var CommentDialog = class {
  constructor(ui, outbox) {
    this.ui = ui;
    this.outbox = outbox;
    const { dialog, form, textarea, cancel } = ui;
    const endMove = moveBy(ui.dialogHeader, dialog, (point) => {
      this.dropped = point;
      this.place();
    }, () => {
      const previous = this.dropped;
      return () => {
        this.dropped = previous;
        this.place();
      };
    });
    new ResizeObserver(() => {
      if (dialog.open) this.place();
    }).observe(dialog);
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.submit();
    });
    textarea.addEventListener("input", () => {
      this.keep();
    });
    textarea.addEventListener("keydown", (event) => {
      if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
        event.preventDefault();
        form.requestSubmit();
      }
    });
    ui.attach.addEventListener("click", () => {
      ui.attachmentInput.click();
    });
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
    cancel.addEventListener("click", () => {
      this.dismiss();
    });
    dialog.addEventListener("cancel", (event) => {
      if (event.target !== dialog) return;
      event.preventDefault();
      this.dismiss();
    });
    dialog.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const rect = dialog.getBoundingClientRect();
      if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) this.dismiss();
    });
    dialog.addEventListener("close", () => {
      endMove();
      this.stopTracking();
      ui.highlight.style.display = "none";
    });
  }
  ui;
  outbox;
  onOpen = () => void 0;
  onClose = () => void 0;
  files = [];
  editing;
  originals = [];
  missingFiles = 0;
  dropped;
  position = { x: 0, y: 0 };
  captured = null;
  /** What the dialog was opened on, which a draft keeps. */
  target = { kind: "page", selector: "" };
  /** The target was gone after a reload: a page comment now. */
  unavailable = false;
  /** Saving this draft in the outbox. */
  sending = false;
  /** A live reload waiting for the dialog to close. */
  pendingReload = false;
  addFiles(files) {
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
  showFiles() {
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
  get open() {
    return this.ui.dialog.open;
  }
  /**
   * Closes the dialog at the person's request (Cancel, Escape, a click
   * outside), unless its comment is being saved locally.
   */
  dismiss() {
    if (!this.sending) this.close();
  }
  /** Reloads now, or when the dialog closes if it is open. */
  reloadWhenClosed() {
    if (this.ui.dialog.open) {
      this.pendingReload = true;
      this.ui.reloadNotice.hidden = false;
    } else {
      window.location.reload();
    }
  }
  /** Opens on `target`; `selector` names it when the page cannot (an answer). */
  openElement(target, x, y, selector) {
    const kind = selector?.startsWith(AGENT_TARGET_PREFIX) ? "answer" : "element";
    const captured = captureTarget(target, selector);
    this.show(captured, x, y, { kind, selector: captured.selector });
  }
  openPdf(captured, x, y) {
    this.show(captured, x, y, { kind: "pdf", selector: captured.selector });
  }
  openPage() {
    const x = window.innerWidth / 2;
    const y = window.innerHeight / 3;
    this.show(capturePageTarget(), x, y, { kind: "page", selector: "" });
  }
  /**
   * Opens the dialog again on the draft this page left, if any, its target
   * found by `find`. A target that is gone makes it a page comment, for
   * good: it stays one after further reloads, and the dialog says so.
   */
  async restore(find) {
    const draft = loadDraft();
    if (draft === null || this.open) return;
    if (draft.editRef === void 0) {
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
  async edit(entry, find, savedDraft) {
    const { draft } = entry;
    const saved = entry.files === void 0 ? { files: [] } : await this.outbox.savedFiles(entry).catch(
      () => ({ files: [] })
    );
    if (this.open || !this.outbox.entries.includes(entry) || entry.state !== "failed") return false;
    const selector = draft.selector;
    const kind = draft.pdf !== void 0 ? "pdf" : selector.startsWith(AGENT_TARGET_PREFIX) ? "answer" : selector === "" ? "page" : "element";
    const refs = draft.attachments ?? [];
    const existing = entry.uploaded && saved.files.length > 0 ? refs.slice(0, -saved.files.length) : refs;
    const originals = [...existing, ...saved.files];
    const files = savedDraft === void 0 ? originals : (savedDraft.originalFiles ?? []).flatMap(
      (index) => originals[index] === void 0 ? [] : [originals[index]]
    );
    const fileCount = savedDraft?.fileCount ?? existing.length + (entry.uploaded === true ? saved.files.length : entry.files?.length ?? 0);
    this.editing = entry;
    this.originals = originals;
    this.showSaved({
      ...draft,
      page: window.location.href,
      target: { kind, selector },
      unavailable: false,
      ...savedDraft,
      fileCount: Math.max(0, fileCount - files.length)
    }, find, files);
    return true;
  }
  showSaved(draft, find, files = []) {
    const { target, selectedText } = draft;
    let element = null;
    if (target.kind !== "page" && target.kind !== "pdf" && !draft.unavailable) {
      try {
        element = find(target.selector);
      } catch {
      }
    }
    const page = draft.page;
    let x = window.innerWidth / 2;
    let y = window.innerHeight / 3;
    let captured = { element: null, page, selector: "", selectedText };
    if (element !== null) {
      captured = { element, page, selector: target.selector, selectedText };
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        x = rect.left;
        y = rect.bottom;
      }
    }
    if (target.kind === "pdf" && draft.pdf !== void 0) {
      captured = { ...captured, selector: target.selector, pdf: draft.pdf };
    }
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
  keep() {
    const captured = this.captured;
    if (captured === null || !this.ui.dialog.open || this.sending) return;
    saveDraft({
      page: captured.page,
      target: this.target,
      unavailable: this.unavailable,
      selectedText: captured.selectedText,
      ...captured.pdf === void 0 ? {} : { pdf: captured.pdf },
      comment: this.ui.textarea.value,
      fileCount: this.files.length + this.missingFiles,
      ...this.editing === void 0 ? {} : {
        editRef: this.editing.ref,
        originalFiles: this.files.map((file) => this.originals.indexOf(file)).filter((index) => index !== -1)
      }
    });
  }
  updateHighlight = () => {
    const { highlight, dialog } = this.ui;
    if (dialog.open) this.place();
    const element = this.captured?.element;
    if (!dialog.open || !element?.isConnected || element.getClientRects().length === 0) {
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
  stopTracking() {
    window.removeEventListener("resize", this.updateHighlight);
    window.visualViewport?.removeEventListener("resize", this.updateHighlight);
    window.visualViewport?.removeEventListener("scroll", this.updateHighlight);
    window.removeEventListener("scroll", this.updateHighlight, true);
  }
  place() {
    const { dialog } = this.ui;
    const { x, y } = this.position;
    const width = dialog.offsetWidth;
    const sideSpace = window.innerWidth - width;
    const left = sideSpace <= 32 ? sideSpace / 2 : x + 12;
    const point = fitPosition(this.dropped ?? { x: left, y: y + 12 }, width, dialog.offsetHeight);
    dialog.style.left = `${point.x}px`;
    dialog.style.top = `${point.y}px`;
  }
  show(captured, x, y, target) {
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
    this.dropped = void 0;
    this.position = { x, y };
    window.getSelection()?.removeAllRanges();
    ui.dialog.showModal();
    this.onOpen(captured);
    this.place();
    this.updateHighlight();
    window.addEventListener("resize", this.updateHighlight);
    window.visualViewport?.addEventListener("resize", this.updateHighlight);
    window.visualViewport?.addEventListener("scroll", this.updateHighlight);
    window.addEventListener("scroll", this.updateHighlight, true);
    ui.textarea.focus();
    this.keep();
  }
  close() {
    const ui = this.ui;
    ui.textarea.blur();
    if (ui.dialog.open) ui.dialog.close();
    window.getSelection()?.removeAllRanges();
    this.onClose();
    this.stopTracking();
    ui.highlight.style.display = "none";
    this.captured = null;
    this.editing = void 0;
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
  setSending(sending) {
    this.sending = sending;
    const { send, cancel, attach, discardMissing, textarea, attachments } = this.ui;
    send.disabled = sending;
    cancel.disabled = sending;
    attach.disabled = sending;
    discardMissing.disabled = sending;
    textarea.readOnly = sending;
    for (const button of attachments.querySelectorAll("button")) button.disabled = sending;
  }
  async submit() {
    const ui = this.ui;
    const captured = this.captured;
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
    const attachments = this.files.filter((file) => !(file instanceof File));
    const draft = {
      page: captured.page,
      selector: captured.selector,
      selectedText: captured.selectedText,
      ...captured.pdf === void 0 ? {} : { pdf: captured.pdf },
      comment,
      ...attachments.length > 0 ? { attachments } : {}
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
};

// client/src/edit.ts
var PRESS_EVENTS = ["pointerdown", "mousedown", "pointerup", "mouseup", "dblclick"];
var EditMode = class {
  constructor(ui, dialog) {
    this.ui = ui;
    this.dialog = dialog;
    ui.edit.addEventListener("click", () => {
      if (this.on) this.leave();
      else this.enter();
    });
    for (const type of PRESS_EVENTS) {
      window.addEventListener(type, (event) => {
        this.swallow(event);
      }, true);
    }
    window.addEventListener("click", (event) => {
      this.select(event);
    }, true);
    document.addEventListener("pointermove", (event) => {
      this.hover(event);
    }, true);
    document.addEventListener("pointerleave", () => {
      this.outline(null);
    }, true);
    window.addEventListener("scroll", () => {
      this.outline(this.hovered);
    }, true);
    window.addEventListener("resize", () => {
      this.outline(this.hovered);
    });
    document.addEventListener("keydown", (event) => {
      if (!this.on || event.key !== "Escape" || this.dialog.open) return;
      event.preventDefault();
      this.leave();
    }, true);
  }
  ui;
  dialog;
  on = false;
  hovered = null;
  /**
   * Called on each press edit mode takes from the page: the document's own
   * listeners (closing the panel and the list on outside presses) do not
   * see it.
   */
  onPagePress = () => void 0;
  enter() {
    this.on = true;
    this.ui.edit.setAttribute("aria-pressed", "true");
  }
  leave() {
    this.on = false;
    this.ui.edit.setAttribute("aria-pressed", "false");
    this.outline(null);
  }
  /** The page element an event is about, or null for jamparty's own overlay. */
  pageTarget(event) {
    if (event.composedPath().includes(this.ui.host)) return null;
    return event.target instanceof Element ? event.target : null;
  }
  primary(event) {
    return !(event instanceof MouseEvent) || event.button === 0;
  }
  /** Keeps a left press from reaching the page: no focus, no selection. */
  swallow(event) {
    if (!this.on || !this.primary(event) || this.pageTarget(event) === null) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    if (event.type === "pointerdown") this.onPagePress();
  }
  /** A left click on the page comments on what was clicked. */
  select(event) {
    if (!this.on || event.button !== 0) return;
    const target = this.pageTarget(event);
    if (target === null) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    this.outline(null);
    this.dialog.openElement(target, event.clientX, event.clientY);
  }
  hover(event) {
    if (!this.on || this.dialog.open) return;
    this.outline(this.pageTarget(event));
  }
  clearHover() {
    this.outline(null);
  }
  outline(target) {
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
};

// client/src/list.ts
function shortSelector(selector) {
  if (selector === "") return "page";
  if (selector.startsWith(AGENT_TARGET_PREFIX)) return "answer";
  const parts = selector.split(" > ");
  return parts[parts.length - 1] ?? selector;
}
function relativeTime(at, now = Date.now()) {
  const minutes = Math.floor((now - at) / 6e4);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
}
function stateText(state) {
  switch (state) {
    case "failed":
      return "refused";
    case "unsent":
      return "not sent";
    case "sending":
      return "sending";
    case "earlier":
      return "sent earlier";
    case "queued":
    case "delivered":
      return state;
  }
}
var CommentList = class {
  constructor(ui, outbox, renderAnswer) {
    this.ui = ui;
    this.outbox = outbox;
    this.renderAnswer = renderAnswer;
    ui.pill.addEventListener("click", () => {
      this.toggle();
    });
    ui.listHeader.addEventListener("click", () => {
      this.close();
    });
    document.addEventListener("pointerdown", (event) => {
      if (this.isOpen && !event.composedPath().includes(ui.list)) this.close();
    }, true);
  }
  ui;
  outbox;
  renderAnswer;
  items = /* @__PURE__ */ new Map();
  replies = /* @__PURE__ */ new Map();
  hovered = null;
  focused = null;
  onPreview = () => void 0;
  onEdit = () => Promise.resolve();
  refreshTimer;
  /** Called when the list opens (the control panel closes then). */
  onOpen = () => void 0;
  get isOpen() {
    return !this.ui.list.hidden;
  }
  toggle() {
    if (this.isOpen) this.close();
    else this.open();
  }
  open() {
    this.onOpen();
    this.ui.list.hidden = false;
    this.ui.bar.hidden = true;
    this.ui.pill.setAttribute("aria-expanded", "true");
    this.render();
    this.refreshTimer = window.setInterval(() => {
      this.render();
    }, 3e4);
    this.ui.collapse.focus();
  }
  close() {
    if (!this.isOpen) return;
    this.ui.list.hidden = true;
    this.hovered = this.focused = null;
    this.onPreview(null);
    this.ui.bar.hidden = false;
    this.ui.pill.setAttribute("aria-expanded", "false");
    if (this.refreshTimer !== void 0) clearInterval(this.refreshTimer);
    this.refreshTimer = void 0;
  }
  /** Shows the connection state in the pill and the list's header. */
  showState(view) {
    const { connection, listDot, connectionText, pill } = this.ui;
    connection.dataset.state = view.colour;
    listDot.dataset.state = view.colour;
    connection.title = view.text;
    connection.setAttribute("aria-label", view.text);
    pill.title = view.text;
    connectionText.dataset.state = view.colour;
    connectionText.textContent = `· ${view.text}`;
  }
  setAnswers(answers) {
    const replies = /* @__PURE__ */ new Map();
    for (const answer of answers) {
      let node = this.replies.get(answer.id)?.node;
      if (node === void 0) {
        node = document.createElement("article");
        node.className = "agent-reply";
        node.dataset.messageId = answer.id;
        this.renderAnswer(answer, node);
      }
      replies.set(answer.id, { answer, node });
    }
    this.replies = replies;
    this.render();
  }
  answerTarget(id) {
    return this.replies.get(id)?.node ?? null;
  }
  createItem(entry) {
    const item = document.createElement("li");
    if (entry.draft.pdf !== void 0) {
      item.tabIndex = 0;
      item.addEventListener("pointerenter", () => {
        this.hovered = entry;
        this.onPreview(entry);
      });
      item.addEventListener("pointerleave", () => {
        this.hovered = null;
        this.onPreview(this.focused?.draft.pdf === void 0 ? null : this.focused);
      });
    } else {
      item.tabIndex = -1;
    }
    item.addEventListener("focusin", () => {
      this.focused = entry;
      if (entry.draft.pdf !== void 0) this.onPreview(entry);
    });
    item.addEventListener("focusout", (event) => {
      if (event.relatedTarget instanceof Node && item.contains(event.relatedTarget)) return;
      this.focused = null;
      this.onPreview(this.hovered);
    });
    this.items.set(entry, item);
    return item;
  }
  render() {
    const { outboxList, answerList } = this.ui;
    const now = Date.now();
    const entries = [...this.outbox.entries].sort((first, second) => second.at - first.at);
    const active = this.ui.shadow.activeElement;
    const focusedReply = active instanceof HTMLElement && active.closest(".agent-reply") !== null;
    const focusedItem = active?.closest("#outbox-list > li");
    const ownComments = new Set(entries.map((entry) => entry.id));
    const byComment = /* @__PURE__ */ new Map();
    const general = [];
    for (const { answer, node } of Array.from(this.replies.values()).reverse()) {
      const id = answer.replyTo;
      if (id !== void 0 && ownComments.has(id)) {
        const replies = byComment.get(id) ?? [];
        replies.push(node);
        byComment.set(id, replies);
      } else {
        const item = document.createElement("li");
        item.append(node);
        general.push(item);
      }
    }
    const focusedAction = active instanceof HTMLButtonElement ? active.textContent : null;
    for (const [entry, item] of this.items) {
      if (entries.includes(entry)) continue;
      item.remove();
      this.items.delete(entry);
      if (this.hovered === entry || this.focused === entry) {
        if (this.hovered === entry) this.hovered = null;
        if (this.focused === entry) this.focused = null;
        this.onPreview(this.hovered ?? (this.focused?.draft.pdf === void 0 ? null : this.focused));
      }
    }
    for (const [index, entry] of entries.entries()) {
      const item = this.items.get(entry) ?? this.createItem(entry);
      const atIndex = outboxList.children.item(index);
      if (atIndex !== item) outboxList.insertBefore(item, atIndex);
      const shown = this.outbox.shown(entry);
      item.dataset.state = shown;
      const row = document.createElement("div");
      row.className = "row";
      const where = document.createElement("span");
      where.className = "where";
      const target = entry.draft.pdf === void 0 ? shortSelector(entry.draft.selector) : `PDF page ${entry.draft.pdf.page}`;
      where.textContent = `${target} · ${relativeTime(entry.at, now)}`;
      where.title = entry.draft.pdf === void 0 ? entry.draft.selector || "the whole page" : target;
      const state = document.createElement("span");
      state.className = "state";
      state.textContent = stateText(shown);
      if (shown === "sending") {
        const spinner = document.createElement("span");
        spinner.className = "sending-indicator";
        spinner.setAttribute("aria-hidden", "true");
        state.prepend(spinner);
      }
      row.append(where, state);
      const text = document.createElement("p");
      text.className = "text";
      text.textContent = entry.draft.comment;
      item.replaceChildren(row, text);
      const names = (entry.draft.attachments ?? []).map((file) => file.name);
      if (entry.uploaded !== true) names.push(...entry.files ?? []);
      for (const name of names) {
        const file = document.createElement("p");
        file.className = "text";
        file.textContent = name;
        item.append(file);
      }
      if (entry.error !== void 0) {
        const error = document.createElement("p");
        error.className = "state";
        error.textContent = entry.error;
        item.append(error);
      }
      if (entry.state === "failed") {
        const edit = document.createElement("button");
        edit.type = "button";
        edit.className = "again";
        edit.textContent = "edit";
        edit.addEventListener("click", () => {
          void this.onEdit(entry);
        });
        const discard = document.createElement("button");
        discard.type = "button";
        discard.className = "again";
        discard.textContent = "discard";
        discard.addEventListener("click", () => {
          this.outbox.discard(entry);
        });
        item.append(edit, discard);
      }
      if (shown !== "sending" && this.outbox.canResend(entry)) {
        const again = document.createElement("button");
        again.type = "button";
        again.className = "again";
        again.textContent = "send again";
        again.addEventListener("click", () => {
          void this.outbox.resend(entry);
        });
        item.append(again);
      }
      if (entry.id !== null) item.append(...byComment.get(entry.id) ?? []);
      if (!focusedReply && item === focusedItem) {
        const button = focusedAction === null ? void 0 : Array.from(item.querySelectorAll("button")).find((candidate) => candidate.textContent === focusedAction);
        (button ?? item).focus({ preventScroll: true });
      }
    }
    answerList.replaceChildren(...general);
    if (focusedReply && active.isConnected) active.focus({ preventScroll: true });
  }
};

// client/src/name.ts
var NamePrompt = class {
  constructor(ui, changeSettings) {
    this.ui = ui;
    this.changeSettings = changeSettings;
    ui.nameForm.addEventListener("submit", (event) => {
      event.preventDefault();
      void this.join();
    });
    ui.nameDialog.addEventListener("cancel", (event) => {
      event.preventDefault();
    });
  }
  ui;
  changeSettings;
  /** Shows or hides the prompt as the settings say. */
  show(settings) {
    const { nameDialog, nameInput, nameError } = this.ui;
    if (!settings.askName) {
      if (nameDialog.open) nameDialog.close();
      return;
    }
    if (nameDialog.open || this.ui.dialog.open) return;
    nameInput.value = settings.name;
    nameError.textContent = "";
    nameDialog.showModal();
    nameInput.focus();
    nameInput.select();
  }
  async join() {
    const { nameInput, nameError, nameJoin } = this.ui;
    nameJoin.disabled = true;
    const failed = failure(await this.changeSettings({ name: nameInput.value.trim() }));
    nameJoin.disabled = false;
    if (failed !== null) {
      nameError.textContent = failed;
      nameInput.focus();
    }
  }
};

// client/src/attachments.ts
function parseAttachments(value) {
  if (!Array.isArray(value) || value.length > MAX_ATTACHMENTS) return null;
  const files = [];
  for (const entry of value) {
    if (!isRecord(entry) || entry.type !== "file" || !isResourceId(entry.file) || typeof entry.name !== "string") return null;
    files.push({ type: "file", file: entry.file, name: entry.name });
  }
  return files;
}
function isResourceId(value) {
  return typeof value === "string" && value.length === 69 && /^sha3:[0-9a-f]{64}$/.test(value);
}
var UploadError = class extends Error {
  constructor(message, transient) {
    super(message);
    this.transient = transient;
  }
  transient;
};
async function upload(file) {
  const reply = await fetch(UPLOAD_URL, {
    method: "POST",
    mode: "same-origin",
    redirect: "error",
    body: file
  }).catch(() => null);
  if (reply === null) throw new UploadError("Could not reach the joiner.", true);
  let result;
  try {
    result = JSON.parse(await reply.text());
  } catch (error) {
    if (!(error instanceof SyntaxError)) throw new UploadError("The upload reply was interrupted.", true);
  }
  if (reply.status === 201 && isRecord(result) && isResourceId(result.id)) {
    return { type: "file", file: result.id, name: file.name };
  }
  const reason = isRecord(result) && typeof result.error === "string" ? result.error : void 0;
  const transient = [408, 429, 502, 503, 504].includes(reply.status) || reason === "could not read the attachment";
  throw new UploadError(reason ?? "Could not upload the attachment.", transient);
}

// client/src/outbox-files.ts
var FILES = "files";
var ACCEPTED = "accepted";
var MAX_AGE = 7 * 24 * 60 * 60 * 1e3;
async function transaction(mode, work) {
  const db = await new Promise((resolve, reject) => {
    const request = indexedDB.open("jamparty-outbox", 2);
    request.onupgradeneeded = (event) => {
      if (event.oldVersion === 0) {
        request.result.createObjectStore(FILES).createIndex("at", "at");
      }
      request.result.createObjectStore(ACCEPTED).createIndex("at", "at");
    };
    request.onsuccess = () => {
      resolve(request.result);
    };
    request.onerror = () => {
      reject(new Error("Could not open attachment storage.", { cause: request.error }));
    };
  });
  try {
    return await new Promise((resolve, reject) => {
      const tx = db.transaction([FILES, ACCEPTED], mode);
      let result;
      tx.oncomplete = () => {
        resolve(result);
      };
      tx.onabort = () => {
        reject(new Error("Attachment storage transaction aborted.", { cause: tx.error }));
      };
      work(tx, (value) => {
        result = value;
      });
    });
  } finally {
    db.close();
  }
}
function valid(files) {
  return Array.isArray(files) && files.length > 0 && files.length <= MAX_ATTACHMENTS && files.every((file) => file instanceof File && file.size <= MAX_FILE_BYTES);
}
function prune(store, done) {
  const expired = store.index("at").openCursor(IDBKeyRange.upperBound(Date.now() - MAX_AGE));
  expired.onsuccess = () => {
    const cursor = expired.result;
    if (cursor === null) {
      done();
    } else {
      cursor.delete();
      cursor.continue();
    }
  };
}
function pruneFiles() {
  void transaction("readwrite", (tx, finish) => {
    prune(tx.objectStore(FILES), () => {
      finish(void 0);
    });
    prune(tx.objectStore(ACCEPTED), () => {
      finish(void 0);
    });
  }).catch(() => void 0);
}
function keepFiles(ref, files, replaces) {
  if (!valid(files)) return Promise.resolve(false);
  return transaction("readwrite", (tx, finish) => {
    const store = tx.objectStore(FILES);
    prune(store, () => {
      const count = store.count();
      const replaced = replaces === void 0 ? null : store.getKey(replaces);
      const check = () => {
        if (count.result - Number(replaced?.result !== void 0) >= MAX_OUTBOX) {
          finish(false);
          return;
        }
        store.put({ at: Date.now(), files }, ref);
        finish(true);
      };
      if (replaced === null) count.onsuccess = check;
      else replaced.onsuccess = check;
    });
  });
}
function readFiles(ref) {
  return transaction("readonly", (tx, finish) => {
    const saved = tx.objectStore(FILES).get(ref);
    const receipt = tx.objectStore(ACCEPTED).get(ref);
    receipt.onsuccess = () => {
      const record = saved.result;
      const accepted = receipt.result;
      finish(accepted === void 0 ? { files: valid(record?.files) ? record.files : [] } : { files: [], accepted });
    };
  });
}
function forgetFiles(ref, accepted) {
  return transaction("readwrite", (tx, finish) => {
    if (accepted !== void 0) {
      tx.objectStore(ACCEPTED).put({ at: Date.now(), ...accepted }, ref);
    }
    tx.objectStore(FILES).delete(ref);
    finish(void 0);
  }).catch(() => {
  });
}

// client/src/outbox.ts
function parseDraft(value) {
  if (!isRecord(value)) return null;
  const { page, selector, selectedText, comment, choice } = value;
  if (typeof page !== "string" || typeof selector !== "string" || typeof comment !== "string") {
    return null;
  }
  const attachments = value.attachments === void 0 ? [] : parseAttachments(value.attachments);
  if (attachments === null) return null;
  const pdf = parsePdfAnchor(value.pdf);
  return {
    page,
    ...pdf === void 0 ? {} : { pdf },
    selector,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment,
    ...attachments.length > 0 ? { attachments } : {},
    ...typeof choice === "string" ? { choice } : {}
  };
}
function parseState(value) {
  switch (value) {
    case "queued":
    case "delivered":
    case "failed":
    case "sending":
      return value;
    default:
      return "unsent";
  }
}
function parseEntry(value) {
  if (!isRecord(value)) return null;
  const draft = parseDraft(value.draft);
  if (draft === null) return null;
  const progress = parseAttachments(value.uploadProgress);
  return {
    draft,
    ref: typeof value.ref === "string" ? value.ref : crypto.randomUUID(),
    id: typeof value.id === "string" ? value.id : null,
    state: parseState(value.state),
    key: typeof value.key === "string" ? value.key : null,
    ...typeof value.error === "string" ? { error: value.error } : {},
    ...Array.isArray(value.files) && value.files.every((name) => typeof name === "string") ? { files: value.files } : {},
    ...progress === null ? {} : { uploadProgress: progress },
    ...value.uploaded === true ? { uploaded: true } : {},
    at: typeof value.at === "number" ? value.at : Date.now()
  };
}
function lastKey() {
  try {
    return sessionStorage.getItem(JOINER_KEY);
  } catch {
    return null;
  }
}
function load() {
  try {
    const stored = JSON.parse(sessionStorage.getItem(OUTBOX_KEY) ?? "[]");
    if (!Array.isArray(stored)) return [];
    return stored.map(parseEntry).filter((entry) => entry !== null);
  } catch {
    return [];
  }
}
var Outbox = class {
  constructor(send, reachable) {
    this.send = send;
    this.reachable = reachable;
    pruneFiles();
  }
  send;
  reachable;
  entries = load();
  /** Called after every change, to redraw. */
  onChange = () => void 0;
  onInitialOutcome = () => void 0;
  active = /* @__PURE__ */ new Set();
  /** Serializes file transfers within this tab. The joiner also caps concurrent uploads. */
  uploads = Promise.resolve();
  /**
   * The ids of this joiner's comments the share holds, as the joiner last
   * said; null while it does not know (a restarted joiner before the
   * share admitted it).
   */
  undelivered = null;
  /** This joiner's key, once a hello said it. */
  key = null;
  /**
   * The key a comment is made under: this joiner's, or before its hello
   * (a comment written as the page loads), that of the joiner this tab
   * last heard from, which it most likely is.
   */
  stamp = lastKey();
  retryTimer;
  retryUploads() {
    this.retryTimer ??= window.setTimeout(() => {
      this.retryTimer = void 0;
      if (this.reachable()) void this.resendAll();
    }, 2e3);
  }
  /** The joiner's key, from its hello or link (null: not known yet, and kept). */
  setKey(key) {
    if (key === null || key === this.key) return;
    this.key = key;
    this.stamp = key;
    for (const entry of this.entries) {
      if (entry.key === null) entry.key = key;
    }
    try {
      sessionStorage.setItem(JOINER_KEY, key);
    } catch {
    }
    this.changed();
  }
  /** Whether `entry` was made under another joiner key than this one's. */
  foreign(entry) {
    return this.key !== null && entry.key !== this.key;
  }
  /** How `entry` is shown. */
  shown(entry) {
    return (entry.state === "queued" || entry.state === "sending") && this.foreign(entry) ? "earlier" : entry.state;
  }
  persist() {
    sessionStorage.setItem(OUTBOX_KEY, JSON.stringify(this.entries));
  }
  /** Persists state changes and redraws. */
  changed() {
    try {
      this.persist();
    } catch {
    }
    this.onChange();
  }
  /**
   * The oldest entry the page may forget: a delivered one, else a queued
   * one (the share holds it); -1 if every entry is still to be sent.
   */
  droppable() {
    const delivered = this.entries.findIndex((entry) => entry.state === "delivered");
    if (delivered !== -1) return delivered;
    return this.entries.findIndex((entry) => entry.state === "queued");
  }
  /** Whether adding a new comment would displace one the browser must keep. */
  full(replaces) {
    return this.entries.length >= MAX_OUTBOX && (replaces === void 0 || !this.entries.includes(replaces)) && this.droppable() === -1;
  }
  /** A queued entry the share no longer holds was delivered. */
  settle(entry) {
    if (entry.state !== "queued" || entry.id === null || this.undelivered === null) return;
    if (this.foreign(entry)) return;
    if (!this.undelivered.has(entry.id)) entry.state = "delivered";
  }
  /**
   * The joiner's word on which of its comments the share holds (null: it
   * does not know yet, and nothing changes).
   */
  sync(undelivered) {
    this.undelivered = undelivered === null ? null : new Set(undelivered);
    for (const entry of this.entries) this.settle(entry);
    this.changed();
  }
  fullError() {
    if (this.entries.some((entry) => entry.state === "failed")) {
      return "The comment list is full. Edit or discard a refused comment, or wait for a comment to queue.";
    }
    return `${MAX_OUTBOX} comments are waiting to be sent; this one can go once they have`;
  }
  /** Make room and persist the new ref before starting any network work. */
  add(draft, ref, files = [], replaces) {
    if (this.full(replaces)) throw new Error(this.fullError());
    const entry = {
      draft,
      ref,
      id: null,
      state: "sending",
      key: this.stamp,
      at: Date.now(),
      ...files.length === 0 ? {} : { files: files.map((file) => file.name) }
    };
    const replaced = replaces === void 0 ? -1 : this.entries.indexOf(replaces);
    const index = replaced !== -1 ? replaced : this.entries.length >= MAX_OUTBOX ? this.droppable() : -1;
    const [removed] = index === -1 ? [] : this.entries.splice(index, 1);
    this.entries.push(entry);
    try {
      this.persist();
    } catch {
      this.entries.pop();
      if (removed !== void 0) this.entries.splice(index, 0, removed);
      throw new Error("Could not save the comment in this tab.");
    }
    if (removed?.files !== void 0) void forgetFiles(removed.ref);
    this.onChange();
    return entry;
  }
  /** Save a comment locally and start delivery without waiting for the share. */
  async enqueue(draft, files, replaces) {
    if (this.full(replaces)) throw new Error(this.fullError());
    if (replaces?.files !== void 0 && (await this.savedFiles(replaces)).accepted !== void 0) {
      throw new Error("This comment has already been queued.");
    }
    const ref = crypto.randomUUID();
    if (files.length > 0 && !await keepFiles(ref, files, replaces?.ref).catch(() => false)) {
      throw new Error("Could not save the files in this browser. Free space or remove an attachment.");
    }
    let entry;
    try {
      entry = this.add(draft, ref, files, replaces);
    } catch (error) {
      if (files.length > 0) await forgetFiles(ref);
      throw error;
    }
    void this.deliver(entry).then((outcome) => {
      if (!this.foreign(entry)) this.onInitialOutcome(outcome);
    });
  }
  /** Choice buttons still wait for a reply before dismissing the question. */
  async submit(draft, ref) {
    let entry;
    try {
      entry = this.add(draft, ref);
    } catch (error) {
      return { refused: { error: error instanceof Error ? error.message : "Could not save the choice." } };
    }
    return this.deliver(entry);
  }
  discard(entry) {
    const index = this.entries.indexOf(entry);
    if (index === -1) return;
    this.entries.splice(index, 1);
    if (entry.files !== void 0) void forgetFiles(entry.ref);
    this.changed();
  }
  accept(entry, id) {
    entry.id = id;
    entry.state = "queued";
    delete entry.error;
    delete entry.files;
    delete entry.uploaded;
    delete entry.uploadProgress;
    this.settle(entry);
  }
  async savedFiles(entry) {
    const saved = await readFiles(entry.ref);
    if (saved.accepted !== void 0) {
      entry.draft.attachments = saved.accepted.attachments;
      this.accept(entry, saved.accepted.id);
      this.changed();
    }
    return saved;
  }
  async uploadFiles(entry, files) {
    if (files.length !== entry.files?.length) {
      throw new UploadError("The saved files are unavailable. Attach them again.", false);
    }
    const uploaded = entry.uploadProgress ?? [];
    entry.uploadProgress = uploaded;
    for (const file of files.slice(uploaded.length)) {
      try {
        uploaded.push(await upload(file));
        this.changed();
      } catch (error) {
        const reason = error instanceof Error ? error.message : "Could not upload the attachment.";
        throw new UploadError(`${file.name} — ${reason}`, error instanceof UploadError && error.transient);
      }
    }
    entry.draft.attachments = [...entry.draft.attachments ?? [], ...uploaded];
    entry.uploaded = true;
    delete entry.uploadProgress;
    this.changed();
  }
  async sendEntry(entry) {
    if (entry.files !== void 0) {
      const work = this.uploads.then(async () => {
        const { files, accepted } = await readFiles(entry.ref);
        if (accepted === void 0 && entry.uploaded !== true && !this.foreign(entry)) {
          await this.uploadFiles(entry, files);
        }
        return accepted;
      });
      this.uploads = work.then(() => void 0, () => void 0);
      try {
        const accepted = await work;
        if (accepted !== void 0) {
          entry.draft.attachments = accepted.attachments;
          return { commented: { id: accepted.id } };
        }
      } catch (error) {
        if (!(error instanceof UploadError) || error.transient) {
          this.retryUploads();
          return { unsent: { error: error instanceof Error ? error.message : "Could not read the files." } };
        }
        return { refused: { error: error.message } };
      }
    }
    if (this.foreign(entry)) return { unsent: { error: "The joiner restarted." } };
    return this.send({ ref: entry.ref, ...entry.draft });
  }
  async deliver(entry) {
    this.active.add(entry);
    let outcome;
    try {
      outcome = await this.sendEntry(entry);
    } catch {
      outcome = { unsent: { error: "Could not send the comment." } };
    }
    if ("refused" in outcome) {
      if (entry.draft.choice !== void 0 && outcome.refused.choiceRefused === true) {
        this.active.delete(entry);
        this.discard(entry);
        return outcome;
      }
      entry.state = entry.draft.choice === void 0 ? "failed" : "unsent";
      entry.error = outcome.refused.error;
    } else if ("commented" in outcome) {
      if (entry.files !== void 0) {
        await forgetFiles(entry.ref, { id: outcome.commented.id, attachments: entry.draft.attachments ?? [] });
      }
      this.accept(entry, outcome.commented.id);
    } else if (!this.foreign(entry)) {
      entry.state = "unsent";
    }
    this.active.delete(entry);
    this.changed();
    return outcome;
  }
  canResend(entry) {
    return !this.active.has(entry) && (entry.state === "unsent" || entry.state === "sending");
  }
  /** Send again under the same ref. Only a manual retry can adopt an earlier key. */
  async resend(entry) {
    if (!this.canResend(entry)) return;
    entry.key = this.stamp;
    entry.state = "sending";
    const work = this.deliver(entry);
    this.changed();
    await work;
  }
  /** Sends every unsent or interrupted entry made under this joiner key. */
  async resendAll() {
    const unsent = this.entries.filter(
      (entry) => this.canResend(entry) && !this.foreign(entry)
    );
    for (const entry of unsent) {
      if (!this.foreign(entry)) await this.resend(entry);
    }
  }
};

// client/src/api.ts
function parseMessage(data) {
  if (typeof data !== "string") return null;
  try {
    const message = JSON.parse(data);
    return isRecord(message) && typeof message.type === "string" ? message : null;
  } catch {
    return null;
  }
}

// client/src/socket.ts
var UNREACHABLE = "jamparty join is not reachable";
var COMMENT_LOST = { unsent: { error: UNREACHABLE } };
var SETTINGS_LOST = { refused: { error: `not saved: ${UNREACHABLE}` } };
var Waiting = class {
  constructor(lost) {
    this.lost = lost;
  }
  lost;
  settles = /* @__PURE__ */ new Map();
  add(n, settle) {
    this.settles.set(n, settle);
  }
  settle(n, outcome) {
    const settle = this.settles.get(n);
    this.settles.delete(n);
    settle?.(outcome);
  }
  /** Every request fails with the socket. */
  lose() {
    for (const settle of this.settles.values()) settle(this.lost);
    this.settles.clear();
  }
};
var PageSocket = class {
  constructor(handlers) {
    this.handlers = handlers;
    window.setInterval(() => {
      if (this.socket !== null && Date.now() - this.lastHeard > WATCHDOG_MS) this.lost();
    }, 5e3);
    window.addEventListener("pagehide", () => {
      this.drop();
    });
    window.addEventListener("pageshow", (event) => {
      if (event.persisted) this.reconnect(0);
    });
  }
  handlers;
  socket = null;
  /** The hello came: the socket carries the joiner's state. */
  ready = false;
  n = 0;
  comments = new Waiting(COMMENT_LOST);
  changes = new Waiting(SETTINGS_LOST);
  lastHeard = Date.now();
  wait = RECONNECT_MIN_MS;
  timer;
  /** Whether the socket carries the joiner's state now. */
  get isOpen() {
    return this.ready;
  }
  /** Opens the socket. */
  start() {
    this.connect();
  }
  connect() {
    if (this.socket !== null) return;
    const socket = new WebSocket(SOCKET_URL);
    this.socket = socket;
    this.lastHeard = Date.now();
    socket.addEventListener("message", (event) => {
      if (this.socket === socket) this.receive(parseMessage(event.data));
    });
    socket.addEventListener("close", () => {
      if (this.socket === socket) this.lost();
    });
  }
  receive(message) {
    this.lastHeard = Date.now();
    if (message === null) return;
    switch (message.type) {
      case "hello": {
        this.wait = RECONNECT_MIN_MS;
        this.ready = true;
        this.handlers.hello(message);
        this.handlers.open(true);
        break;
      }
      case "comment_reply":
        this.comments.settle(message.to, message.outcome);
        break;
      case "settings_reply":
        this.changes.settle(message.to, message.outcome);
        break;
      case "ping":
        break;
      case "link":
      case "people":
      case "mine":
      case "settings":
      case "reload":
      case "answer_resolved":
      case "answer":
      case "cursor":
        if (this.ready) this.handlers.slice(message);
        break;
    }
  }
  /** Sends `message` if the socket is up; false if it is not. */
  send(message) {
    const socket = this.socket;
    if (!this.ready || socket?.readyState !== WebSocket.OPEN) return false;
    socket.send(JSON.stringify(message));
    return true;
  }
  /** Hands over a comment; its outcome ("unsent" if the socket goes first). */
  comment(submission) {
    return this.request(this.comments, (n) => ({ type: "comment", ...submission, n }));
  }
  /** Changes the settings; the outcome ("refused" if the socket goes first). */
  changeSettings(change) {
    return this.request(this.changes, (n) => ({ type: "settings", change, n }));
  }
  request(waiting, message) {
    this.n += 1;
    const n = this.n;
    return new Promise((resolve) => {
      if (this.send(message(n))) waiting.add(n, resolve);
      else resolve(waiting.lost);
    });
  }
  /** Closes the socket without waiting for the joiner to agree. */
  drop() {
    const socket = this.socket;
    this.socket = null;
    if (this.timer !== void 0) clearTimeout(this.timer);
    this.timer = void 0;
    if (socket !== null) socket.close();
    const wasReady = this.ready;
    this.ready = false;
    this.comments.lose();
    this.changes.lose();
    if (wasReady) this.handlers.open(false);
  }
  /** The socket closed or went silent: shown at once, opened again soon. */
  lost() {
    this.drop();
    this.backOff();
  }
  /** Tries again after the current wait, and doubles it for the next. */
  backOff() {
    this.reconnect(this.wait);
    this.wait = Math.min(this.wait * 2, RECONNECT_MAX_MS);
  }
  reconnect(wait) {
    if (this.timer !== void 0) return;
    this.timer = window.setTimeout(() => {
      this.timer = void 0;
      this.connect();
    }, wait);
  }
};

// client/src/styles.ts
var dark = `
    color-scheme: dark;
    --jp-surface: #161616;
    --jp-surface-header: #24272d;
    --jp-field: #202124;
    --jp-hover: #303338;
    --jp-border: #474b52;
    --jp-input-border: #717780;
    --jp-text: #f5f6f8;
    --jp-muted: #b9c0cb;
    --jp-link: #9ac8ff;
    --jp-focus: #84b9ff;
    --jp-selected: #253d60;
    --jp-warning: #f0c27a;
    --jp-error: #ffa8a8;
    --jp-success: #8ce99a;
    --jp-shadow: 0 calc(8 * var(--jp-unit)) calc(24 * var(--jp-unit)) rgb(0 0 0 / 26%);
  `;
var styles = `
  :host { all: initial; }

  #overlay-root {
    all: initial;
    display: contents;
    color-scheme: light;
    /* --jp-unit is set on this root by Ui from the validated text size. */
    --jp-font: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    font-family: var(--jp-font);
    --jp-surface: #fff;
    --jp-surface-header: #f0f3f7;
    --jp-field: #f6f7fa;
    --jp-hover: #e9edf4;
    --jp-border: #d5dbe5;
    --jp-input-border: #7b8494;
    --jp-text: #202632;
    --jp-muted: #535f71;
    --jp-link: #1456ad;
    --jp-focus: #246bcb;
    --jp-selected: #dceaff;
    --jp-warning: #784b11;
    --jp-error: #a22222;
    --jp-success: #176630;
    --jp-shadow: 0 calc(8 * var(--jp-unit)) calc(24 * var(--jp-unit)) rgb(28 40 60 / 16%);
    --jp-accent: #225fbe;
    --jp-on-accent: #fff;
    --jp-anchor: #2479e9;
    --jp-anchor-active: #1456ad;
    --jp-tint: rgb(47 128 237 / 8%);
    --jp-connected: #2f9e44;
    --jp-switch-on: #288337;
    --jp-reconnecting: #e8a33d;
    --jp-disconnected: #9a958e;
    --jp-cursor-text: #fff;
    --jp-radius: calc(11 * var(--jp-unit));
    font-size: calc(13 * var(--jp-unit));
  }

  :host([data-theme="dark"]) #overlay-root { ${dark} }
  @media (prefers-color-scheme: dark) {
    :host(:not([data-theme="light"])) #overlay-root { ${dark} }
  }

  *, *::before, *::after { box-sizing: border-box; }

  #highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid var(--jp-anchor);
    background: var(--jp-tint);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 90%);
    pointer-events: none;
  }

  #edit-highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid var(--jp-anchor);
    background: var(--jp-tint);
    pointer-events: none;
  }

  #bar {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(14 * var(--jp-unit));
    z-index: 2147483644;
    display: flex;
    align-items: center;
    gap: calc(6 * var(--jp-unit));
  }

  #bar[hidden] { display: none; }

  #pill {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--jp-unit));
    padding: calc(7 * var(--jp-unit)) calc(12 * var(--jp-unit)) calc(7 * var(--jp-unit)) calc(9 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 999px;
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    cursor: pointer;
    font: 600 calc(12 * var(--jp-unit))/1 var(--jp-font);
  }

  .dot {
    display: block;
    flex: none;
    width: calc(10 * var(--jp-unit));
    height: calc(10 * var(--jp-unit));
    border-radius: 50%;
    background: var(--jp-disconnected);
  }

  .dot[data-state="green"] { background: var(--jp-connected); }
  .dot[data-state="amber"] { background: var(--jp-reconnecting); }
  .dot[data-state="grey"] { background: var(--jp-disconnected); }

  #gear, #edit, #pdf-outline, #attach {
    display: flex;
    align-items: center;
    justify-content: center;
    width: calc(32 * var(--jp-unit));
    height: calc(32 * var(--jp-unit));
    padding: 0;
    border: 1px solid var(--jp-border);
    border-radius: 50%;
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    cursor: pointer;
  }

  :is(#pill, #gear, #edit, #pdf-outline, #attach):hover { background: var(--jp-hover); }
  #pdf-outline[hidden] { display: none; }
  #gear[aria-expanded="true"], #pdf-outline[aria-expanded="true"] { background: var(--jp-hover); }
  #edit[aria-pressed="true"] { border-color: var(--jp-accent); background: var(--jp-accent); color: var(--jp-on-accent); }

  #list {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(14 * var(--jp-unit));
    z-index: 2147483644;
    display: flex;
    flex-direction: column;
    width: min(calc(440 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit)));
    max-height: min(60vh, calc(100vh - 28 * var(--jp-unit)));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(12 * var(--jp-unit))/1.45 var(--jp-font);
    overflow: hidden;
  }

  #list[hidden] { display: none; }

  #list header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: calc(10 * var(--jp-unit));
    padding: calc(12 * var(--jp-unit)) calc(14 * var(--jp-unit));
    border-bottom: 1px solid var(--jp-border);
    cursor: pointer;
  }

  #list-summary {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--jp-unit));
    min-width: 0;
    font-weight: 600;
  }

  #connection-text {
    overflow: hidden;
    color: var(--jp-muted);
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  #connection-text[data-state="grey"], #connection-text[data-state="amber"] { color: var(--jp-warning); }

  #collapse {
    flex: none;
    width: calc(28 * var(--jp-unit));
    height: calc(28 * var(--jp-unit));
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--jp-muted);
    font: calc(16 * var(--jp-unit))/1 var(--jp-font);
  }

  #collapse:hover, #collapse:focus-visible { background: var(--jp-hover); }

  #list-content { overflow: auto; }

  #outbox-list, #answer-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: calc(4 * var(--jp-unit)) 0;
    list-style: none;
  }

  #outbox-list > li, #answer-list > li {
    display: flex;
    flex-direction: column;
    gap: calc(4 * var(--jp-unit));
    padding: calc(10 * var(--jp-unit)) calc(14 * var(--jp-unit));
    border-bottom: 1px solid var(--jp-border);
  }

  #outbox-list > li:last-child, #answer-list > li:last-child { border-bottom: 0; }
  #outbox-list:empty, #answer-list:empty { display: none; }
  #outbox-list:not(:empty) + #answer-list:not(:empty) { border-top: 1px solid var(--jp-border); }

  .agent-reply { min-width: 0; color: var(--jp-text); overflow-wrap: anywhere; }
  #outbox-list .agent-reply {
    margin-top: calc(4 * var(--jp-unit));
    padding-inline-start: calc(8 * var(--jp-unit));
    border-inline-start: 2px solid var(--jp-border);
  }
  .agent-reply p { margin: 0 0 calc(4 * var(--jp-unit)); }
  .agent-reply p:last-child { margin-bottom: 0; }
  .agent-reply a { color: var(--jp-link); }
  .agent-reply pre { overflow: auto; }
  .agent-reply table { display: block; overflow: auto; }
  #outbox-list .row { display: flex; justify-content: space-between; gap: calc(10 * var(--jp-unit)); }
  #outbox-list .where, #outbox-list .state { color: var(--jp-muted); }
  #outbox-list .where { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #outbox-list li[data-state="delivered"] .state { color: var(--jp-success); }
  .sending-indicator {
    display: inline-block;
    width: .65em; height: .65em;
    margin-right: .45em;
    border: 1px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: jamparty-sending .8s linear infinite;
  }
  @keyframes jamparty-sending { to { transform: rotate(1turn); } }
  @media (prefers-reduced-motion: reduce) {
    .sending-indicator { animation: none; border-right-color: currentColor; }
  }
  #outbox-list li:is([data-state="unsent"], [data-state="failed"]) .state { color: var(--jp-error); }
  #outbox-list li[data-state="delivered"] .text,
  #outbox-list li[data-state="unsent"] .text { color: var(--jp-text); }
  #outbox-list .text { margin: 0; overflow-wrap: anywhere; white-space: pre-wrap; }

  #outbox-list .again {
    align-self: flex-start;
    padding: calc(3 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 4px;
    background: var(--jp-hover);
    color: var(--jp-text);
    font: calc(11 * var(--jp-unit)) var(--jp-font);
  }


  #panel, #pdf-sections {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(58 * var(--jp-unit));
    z-index: 2147483644;
    width: min(calc(320 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit)));
    max-height: calc(100dvh - 72 * var(--jp-unit));
    overflow: auto;
    padding: calc(6 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(12 * var(--jp-unit))/1.4 var(--jp-font);
  }

  #panel[hidden], #pdf-sections[hidden] { display: none; }

  #pdf-sections {
    max-height: min(60vh, calc(100vh - 72 * var(--jp-unit)));
    overflow: auto;
    padding: calc(10 * var(--jp-unit));
  }

  #pdf-sections h2 { margin: 0 calc(6 * var(--jp-unit)) calc(6 * var(--jp-unit)); font: inherit; font-weight: 600; }
  #pdf-sections ul { list-style: none; padding: 0; margin: 0; }
  #pdf-sections li > ul { margin-inline-start: calc(12 * var(--jp-unit)); }
  .pdf-section-row { display: flex; align-items: flex-start; }
  .pdf-section-row button {
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
    padding: calc(5 * var(--jp-unit)) calc(6 * var(--jp-unit));
    cursor: pointer;
  }
  .pdf-section-link { flex: 1; min-width: 0; overflow-wrap: anywhere; }
  .pdf-section-row button:disabled { color: var(--jp-muted); cursor: default; }
  .pdf-section-row button:not(:disabled):hover { background: var(--jp-hover); }
  .pdf-section-row button[aria-current="location"] { background: var(--jp-selected); }
  :is(#pdf-outline, #pdf-sections button):focus-visible {
    outline: 2px solid var(--jp-link);
    outline-offset: -2px;
  }

  .control {
    display: flex;
    align-items: center;
    gap: calc(8 * var(--jp-unit));
    padding: calc(9 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border-radius: 6px;
  }

  #text-size-row, #text-size-row .segments { flex-wrap: wrap; }

  .control .label { flex-grow: 1; cursor: pointer; }

  .segments {
    display: flex;
    gap: calc(2 * var(--jp-unit));
    padding: calc(2 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 7px;
    background: var(--jp-field);
  }

  .segment {
    min-height: calc(24 * var(--jp-unit));
    padding: calc(3 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--jp-muted);
    cursor: pointer;
    font: 600 calc(12 * var(--jp-unit))/1 var(--jp-font);
  }

  .corner { width: calc(25 * var(--jp-unit)); padding: 0; font-size: calc(14 * var(--jp-unit)); }
  .segment:hover { background: var(--jp-hover); }
  .segment[aria-pressed="true"] { background: var(--jp-accent); color: var(--jp-on-accent); }

  /* The chosen corner (Panel sets data-corner on the host): what sits at
     the bottom right by default moves to that corner. */
  :host([data-corner$="left"]) :is(#bar, #list, #panel, #pdf-sections, #toast) {
    right: auto;
    left: calc(14 * var(--jp-unit));
  }

  :host([data-corner^="top"]) :is(#bar, #list) { bottom: auto; top: calc(14 * var(--jp-unit)); }
  :host([data-corner^="top"]) :is(#panel, #pdf-sections) { bottom: auto; top: calc(58 * var(--jp-unit)); }
  :host([data-corner^="top"]) #toast { bottom: auto; top: calc(52 * var(--jp-unit)); }
  :host([data-corner$="left"]) #agent-general { right: auto; left: 0; align-items: flex-start; }
  :host([data-corner^="top"]) #agent-general { bottom: auto; top: calc(40 * var(--jp-unit) + var(--sections-offset, 0px)); }

  .chip {
    padding: 1px calc(5 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    cursor: pointer;
    font: calc(11 * var(--jp-unit))/1.3 var(--jp-font);
    white-space: nowrap;
  }

  .chip[data-listening="true"] {
    padding: 1px calc(6 * var(--jp-unit));
    border: 1px dashed var(--jp-focus);
    color: var(--jp-focus);
  }

  .switch {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: flex-start;
    width: calc(28 * var(--jp-unit));
    height: calc(16 * var(--jp-unit));
    padding: calc(2 * var(--jp-unit));
    border: 0;
    border-radius: 999px;
    background: var(--jp-border);
    cursor: pointer;
  }

  .switch::after {
    content: "";
    display: block;
    width: calc(12 * var(--jp-unit));
    height: calc(12 * var(--jp-unit));
    border-radius: 50%;
    background: var(--jp-muted);
  }

  .switch[aria-checked="true"] { justify-content: flex-end; background: var(--jp-switch-on); }
  .switch[aria-checked="true"]::after { background: var(--jp-on-accent); }

  #name-row { display: flex; align-items: center; gap: calc(8 * var(--jp-unit)); padding: calc(6 * var(--jp-unit)) calc(10 * var(--jp-unit)); }
  #name-row label { color: var(--jp-muted); font-weight: 400; margin: 0; }

  #name-field {
    flex-grow: 1;
    min-width: 0;
    padding: calc(4 * var(--jp-unit)) calc(6 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 5px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(12 * var(--jp-unit))/1.3 var(--jp-font);
  }

  #name-field:focus { outline: 1px solid var(--jp-focus); }

  #panel-message {
    padding: 0 calc(10 * var(--jp-unit)) calc(6 * var(--jp-unit));
    color: var(--jp-warning);
    font-size: calc(11 * var(--jp-unit));
  }

  #panel-message:empty { display: none; }

  #people {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: calc(10 * var(--jp-unit));
    margin-top: calc(4 * var(--jp-unit));
    padding: calc(10 * var(--jp-unit)) calc(10 * var(--jp-unit)) calc(6 * var(--jp-unit));
    border-top: 1px solid var(--jp-border);
  }

  #here-count { color: var(--jp-muted); font-size: calc(11 * var(--jp-unit)); }

  #avatars {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    align-items: center;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .avatar {
    display: flex;
    align-items: center;
    justify-content: center;
    width: calc(24 * var(--jp-unit));
    height: calc(24 * var(--jp-unit));
    border: 2px solid var(--jp-surface);
    border-radius: 50%;
    color: var(--jp-cursor-text);
    font: 700 calc(10 * var(--jp-unit))/1 var(--jp-font);
  }

  .avatar + .avatar { margin-left: calc(-6 * var(--jp-unit)); }
  .avatar[data-you="true"] {
    border-color: var(--jp-text);
    background: var(--jp-surface);
    color: var(--jp-text);
  }

  #cursors {
    position: fixed;
    inset: 0;
    z-index: 2147483642;
    overflow: hidden;
    pointer-events: none;
  }

  .cursor {
    position: absolute;
    left: 0;
    top: 0;
    width: calc(120 * var(--jp-unit));
    height: calc(48 * var(--jp-unit));
    opacity: 1;
    transition: opacity 400ms;
    will-change: transform;
  }

  .cursor[data-faded="true"] { opacity: 0; }
  .cursor svg { position: absolute; left: 0; top: 0; filter: drop-shadow(0 1px calc(2 * var(--jp-unit)) rgb(0 0 0 / 30%)); }

  .cursor .tag {
    position: absolute;
    left: calc(13 * var(--jp-unit));
    top: calc(17 * var(--jp-unit));
    padding: calc(3 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border-radius: 0 calc(6 * var(--jp-unit)) calc(6 * var(--jp-unit)) calc(6 * var(--jp-unit));
    color: var(--jp-cursor-text);
    box-shadow: 0 1px calc(3 * var(--jp-unit)) rgb(0 0 0 / 25%);
    font: 600 calc(11 * var(--jp-unit))/1.3 var(--jp-font);
    white-space: nowrap;
  }

  #name-dialog {
    top: 50%;
    left: 50%;
    width: min(calc(400 * var(--jp-unit)), calc(100vw - 24 * var(--jp-unit)));
    padding: calc(22 * var(--jp-unit));
    border-radius: var(--jp-radius);
    font: calc(13 * var(--jp-unit))/1.45 var(--jp-font);
    transform: translate(-50%, -50%);
  }

  #name-dialog::backdrop { background: rgb(0 0 0 / 35%); }
  #name-dialog form { display: flex; flex-direction: column; gap: calc(14 * var(--jp-unit)); padding: 0; }
  #name-title { display: flex; flex-direction: column; gap: calc(4 * var(--jp-unit)); }
  #name-title strong { font: 700 calc(20 * var(--jp-unit)) var(--jp-font); }
  #name-title span, #name-foot span { color: var(--jp-muted); }
  #name-foot { display: flex; align-items: center; justify-content: space-between; gap: calc(10 * var(--jp-unit)); }
  #name-foot span { font-size: calc(11 * var(--jp-unit)); }
  #name-dialog label { margin: 0; }

  #name-input {
    padding: calc(8 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 4px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(14 * var(--jp-unit))/1.5 var(--jp-font);
  }

  #name-error { color: var(--jp-error); font-size: calc(11 * var(--jp-unit)); }
  #name-error:empty { display: none; }

  #toast {
    position: fixed;
    right: calc(14 * var(--jp-unit));
    bottom: calc(52 * var(--jp-unit));
    z-index: 2147483644;
    max-width: min(calc(360 * var(--jp-unit)), calc(100vw - 28 * var(--jp-unit)));
    padding: calc(9 * var(--jp-unit)) calc(11 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(13 * var(--jp-unit))/1.35 var(--jp-font);
  }

  #toast[hidden] { display: none; }

  #agent-log { display: contents; }

  #agent-general {
    position: fixed;
    right: 0;
    bottom: calc(40 * var(--jp-unit) + var(--sections-offset, 0px));
    z-index: 2147483643;
    display: flex;
    width: min(calc(388 * var(--jp-unit)), 100vw);
    max-height: calc(100dvh - 54 * var(--jp-unit) - var(--sections-offset, 0px));
    flex-direction: column;
    align-items: flex-end;
    gap: calc(8 * var(--jp-unit));
    padding: calc(12 * var(--jp-unit)) calc(14 * var(--jp-unit));
    pointer-events: none;
  }

  #agent-general.scrollable { overflow: auto; }

  #agent-anchored {
    position: absolute;
    top: 0;
    left: 0;
    width: 0;
    height: 0;
    z-index: 2147483643;
    pointer-events: none;
  }

  .agent-message {
    position: relative;
    width: min(calc(340 * var(--jp-unit)), calc(100vw - 16 * var(--jp-unit)));
    max-height: calc(100vh - 16 * var(--jp-unit));
    padding: calc(7 * var(--jp-unit)) calc(12 * var(--jp-unit)) calc(11 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
    font: calc(13 * var(--jp-unit))/1.45 var(--jp-font);
    overflow-wrap: anywhere;
    overflow-y: auto;
    pointer-events: auto;
  }

  .agent-placement { position: absolute; inset: 0; pointer-events: none; }
  .agent-placement.target-offscreen { visibility: hidden; }
  .agent-placement.viewport-anchored { position: fixed; }
  #agent-anchored .agent-message { position: absolute; z-index: 1; }
  #agent-general .agent-message { max-width: 100%; flex: none; }
  #agent-general .manually-placed { position: fixed; }
  .agent-message[hidden], .agent-anchor-marker[hidden], .agent-anchor-layer g[hidden] { display: none; }
  .agent-anchor-marker { position: absolute; pointer-events: none; border-radius: 2px; }
  .agent-anchor-marker.active { background: var(--jp-tint); }
  .agent-anchor-layer {
    position: absolute;
    inset: 0;
    width: 1px;
    height: 1px;
    overflow: visible;
    color: var(--jp-anchor);
    pointer-events: none;
  }
  .agent-anchor-layer .active { color: var(--jp-anchor-active); }
  .agent-anchor-layer path { fill: none; stroke: currentColor; stroke-linecap: round; }
  .agent-anchor-brace { stroke-width: 1.6px; }
  .agent-anchor-line { stroke-width: 1.4px; opacity: .84; }
  .agent-anchor-layer circle { fill: currentColor; }
  .agent-anchor-layer .active .agent-anchor-line { opacity: 1; }
  .agent-message p { margin: 0 0 calc(8 * var(--jp-unit)); }
  .agent-message p:last-child { margin-bottom: 0; }
  .agent-message ul, .agent-message ol { margin: calc(6 * var(--jp-unit)) 0; padding-left: calc(22 * var(--jp-unit)); }
  .agent-message pre { overflow: auto; }
  .agent-message code { font: inherit; background: var(--jp-hover); }
  .agent-show { display: inline-block; margin-top: calc(6 * var(--jp-unit)); }
  .agent-show[hidden] { display: none; }
  .agent-message a { color: var(--jp-link); text-decoration: underline; }
  .agent-message.has-anchor { cursor: pointer; }
  .agent-message.has-anchor:focus-visible {
    outline: 3px solid var(--jp-focus);
    outline-offset: 2px;
  }

  .agent-header { margin: calc(-7 * var(--jp-unit)) calc(-12 * var(--jp-unit)) calc(8 * var(--jp-unit)); }
  #dialog-header { margin: calc(-14 * var(--jp-unit)) calc(-14 * var(--jp-unit)) calc(12 * var(--jp-unit)); }

  :is(.agent-header, #dialog-header) {
    display: flex;
    align-items: center;
    gap: calc(6 * var(--jp-unit));
    padding: calc(4 * var(--jp-unit)) calc(8 * var(--jp-unit));
    border-bottom: 1px solid var(--jp-border);
    border-radius: calc(var(--jp-radius) - 1px) calc(var(--jp-radius) - 1px) 0 0;
    background: var(--jp-surface-header);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  [data-dragging] { cursor: grabbing; }

  .move-control {
    display: grid;
    place-items: center;
    flex: none;
    width: calc(21 * var(--jp-unit));
    height: calc(21 * var(--jp-unit));
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    cursor: inherit;
  }
  .move-control:is(:hover, :focus-visible) {
    background: var(--jp-hover);
    color: var(--jp-text);
  }
  .move-control:focus-visible { outline: 2px solid var(--jp-focus); }

  .agent-dismiss {
    flex: none;
    margin-inline-start: auto;
    width: calc(21 * var(--jp-unit));
    height: calc(21 * var(--jp-unit));
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--jp-muted);
    font: calc(18 * var(--jp-unit))/1 var(--jp-font);
  }

  .agent-dismiss:hover, .agent-dismiss:focus-visible { background: var(--jp-hover); }

  .agent-meta {
    color: var(--jp-muted);
    font-size: calc(11 * var(--jp-unit));
    font-weight: 700;
  }

  .agent-meta[hidden] { display: none; }

  .agent-unavailable {
    margin-top: calc(8 * var(--jp-unit));
    color: var(--jp-muted);
    font-size: calc(11 * var(--jp-unit));
  }

  .agent-unavailable[hidden] { display: none; }

  #message-highlight {
    position: fixed;
    display: none;
    z-index: 2147483641;
    border: 1px solid var(--jp-anchor);
    border-radius: 3px;
    background: var(--jp-tint);
    pointer-events: none;
  }

  dialog {
    position: fixed;
    inset: auto;
    width: min(calc(420 * var(--jp-unit)), calc(100vw - 24 * var(--jp-unit)));
    max-width: calc(100vw - 24 * var(--jp-unit));
    margin: 0;
    padding: 0;
    border: 1px solid var(--jp-border);
    border-radius: var(--jp-radius);
    background: var(--jp-surface);
    color: var(--jp-text);
    box-shadow: var(--jp-shadow);
  }

  dialog::backdrop { background: rgb(0 0 0 / 6%); }
  form { padding: calc(14 * var(--jp-unit)); }

  label {
    display: block;
    margin-bottom: calc(8 * var(--jp-unit));
    font-weight: 700;
    line-height: 1.3;
  }

  #dialog-label { font-size: calc(14 * var(--jp-unit)); margin: 0; flex: 1; }

  #selection-preview {
    margin: 0 0 calc(10 * var(--jp-unit));
    padding: calc(8 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border-left: 3px solid var(--jp-anchor);
    background: var(--jp-hover);
  }

  #selection-preview[hidden] { display: none; }

  #selection-label {
    margin-bottom: calc(4 * var(--jp-unit));
    color: var(--jp-muted);
    font-size: calc(10 * var(--jp-unit));
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  #selection-text {
    max-height: calc(96 * var(--jp-unit));
    margin: 0;
    overflow: auto;
    color: var(--jp-text);
    font: calc(12 * var(--jp-unit))/1.4 var(--jp-font);
    white-space: pre-wrap;
  }

  textarea {
    display: block;
    width: 100%;
    min-height: calc(112 * var(--jp-unit));
    resize: vertical;
    appearance: none;
    box-shadow: none;
    padding: calc(9 * var(--jp-unit)) calc(10 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 6px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(14 * var(--jp-unit))/1.4 var(--jp-font);
  }

  textarea:focus { border-color: transparent; outline: 2px solid var(--jp-focus); outline-offset: -1px; }

  #jamparty-dialog { max-height: calc(100dvh - 24 * var(--jp-unit)); overflow: auto; }

  #attachments {
    display: flex;
    flex-wrap: wrap;
    gap: calc(6 * var(--jp-unit));
    padding: 0;
    margin: calc(8 * var(--jp-unit)) 0 0;
    list-style: none;
  }

  #attachments[hidden], #missing-attachments[hidden] { display: none; }

  #attachments li {
    display: flex;
    align-items: center;
    gap: calc(5 * var(--jp-unit));
    min-width: 0;
    max-width: 100%;
    padding: calc(2 * var(--jp-unit)) calc(4 * var(--jp-unit)) calc(2 * var(--jp-unit)) calc(8 * var(--jp-unit));
    background: var(--jp-hover);
    border-radius: 5px;
    font-size: calc(12 * var(--jp-unit));
  }

  #attachments .attachment-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  #attachments .attachment-size { white-space: nowrap; }

  #attachments button, #discard-missing {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: inherit;
    cursor: pointer;
  }

  #attachments button:hover, #discard-missing:hover { border-color: var(--jp-input-border); }

  #missing-attachments {
    margin-top: calc(8 * var(--jp-unit));
    font-size: calc(12 * var(--jp-unit));
    color: var(--jp-error);
  }

  #discard-missing { text-decoration: underline; }
  #attach { margin-right: auto; box-shadow: none; }
  #cancel { background: transparent; border-color: transparent; }
  #cancel:hover { background: var(--jp-hover); }
  #attach svg { display: block; }

  :is(#gear, #pdf-outline, #attach) svg { width: calc(16 * var(--jp-unit)); height: calc(16 * var(--jp-unit)); }
  #edit svg { width: calc(15 * var(--jp-unit)); height: calc(15 * var(--jp-unit)); }
  .move-control svg { width: calc(12 * var(--jp-unit)); height: calc(14 * var(--jp-unit)); }
  .cursor svg { width: calc(16 * var(--jp-unit)); height: calc(18 * var(--jp-unit)); }

  #status {
    margin-top: calc(7 * var(--jp-unit));
    color: var(--jp-error);
    font-size: calc(11 * var(--jp-unit));
    line-height: 1.4;
  }

  #status:empty { display: none; }

  #reload-notice {
    margin-top: calc(7 * var(--jp-unit));
    color: var(--jp-muted);
    font-size: calc(11 * var(--jp-unit));
    line-height: 1.4;
  }

  #reload-notice[hidden] { display: none; }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: calc(8 * var(--jp-unit));
    margin-top: calc(10 * var(--jp-unit));
  }

  button {
    padding: calc(7 * var(--jp-unit)) calc(11 * var(--jp-unit));
    border: 1px solid var(--jp-border);
    border-radius: 6px;
    background: var(--jp-field);
    color: var(--jp-text);
    cursor: pointer;
    font: 600 calc(12 * var(--jp-unit))/1 var(--jp-font);
  }

  button[type="submit"] { border-color: var(--jp-accent); background: var(--jp-accent); color: var(--jp-on-accent); }
  button:disabled { cursor: wait; opacity: 0.55; }

  .agent-timeout { position: absolute; bottom: 0; left: 0; right: 0; height: calc(4 * var(--jp-unit));
    background: var(--jp-muted); transform-origin: left; pointer-events: none;
    animation: answer-countdown linear forwards; }
  @keyframes answer-countdown { from { transform: scaleX(1); } to { transform: scaleX(0); } }
  @media (prefers-reduced-motion: reduce) {
    .agent-timeout { animation-timing-function: steps(var(--answer-timeout-steps), end); }
  }
  #answer-timeout {
    width: calc(60 * var(--jp-unit));
    min-width: 0;
    padding: calc(4 * var(--jp-unit)) calc(6 * var(--jp-unit));
    border: 1px solid var(--jp-input-border);
    border-radius: 5px;
    background: var(--jp-field);
    color: var(--jp-text);
    font: calc(12 * var(--jp-unit))/1.3 var(--jp-font);
  }

  :is(button, a, input):focus-visible { outline: 2px solid var(--jp-focus); outline-offset: 1px; }

  .agent-choices { display: flex; flex-wrap: wrap; gap: calc(6 * var(--jp-unit)); margin-top: calc(8 * var(--jp-unit)); }
  .agent-choices:empty { display: none; }
  .agent-choices button { border: 1px solid var(--jp-input-border); border-radius: 5px; padding: calc(5 * var(--jp-unit)) calc(9 * var(--jp-unit));
    background: var(--jp-field); color: var(--jp-muted); cursor: pointer; font: inherit; overflow-wrap: anywhere; }
  .agent-choices button:disabled { opacity: .6; cursor: default; }
`;

// client/src/ui.ts
var TEXT_SCALES = {
  small: 1,
  default: 1.125,
  large: 1.25,
  larger: 1.375
};
var GEAR_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"></path></svg>`;
var PEN_ICON = `<svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 3 5 5M3 21l5-1L21 7a2.1 2.1 0 0 0-5-5L3 15z"></path></svg>`;
var PAPERCLIP_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21 11-9 9a5 5 0 0 1-7-7l10-10a3.5 3.5 0 0 1 5 5L10 18a1.5 1.5 0 0 1-2-2l9-9"></path></svg>`;
var MARKUP = `
  <div id="cursors" aria-hidden="true"></div>
  <div id="highlight" aria-hidden="true"></div>
  <div id="edit-highlight" aria-hidden="true"></div>
  <div id="message-highlight" aria-hidden="true"></div>
  <div id="bar">
    <button id="pill" type="button" aria-expanded="false" aria-controls="list">
      <span id="connection" class="dot" data-state="amber" role="img" aria-label="Connecting" title="Connecting"></span>
      <span id="badge" role="status" aria-live="polite">0 queued</span>
    </button>
    <button id="edit" type="button" aria-label="Edit mode" aria-pressed="false" title="Edit mode: click an element to comment on it (Esc leaves)">${PEN_ICON}</button>
    <button id="gear" type="button" aria-label="Settings" aria-expanded="false" aria-controls="panel">${GEAR_ICON}</button>
  </div>
  <section id="list" aria-label="Comments and replies" hidden>
    <header id="list-header">
      <div id="list-summary">
        <span id="list-dot" class="dot" data-state="amber"></span>
        <span id="list-count">0 queued</span>
        <span id="connection-text" role="status">· connecting</span>
      </div>
      <button id="collapse" type="button" aria-label="Collapse">▾</button>
    </header>
    <div id="list-content">
      <ol id="outbox-list"></ol>
      <ul id="answer-list" aria-label="Replies"></ul>
    </div>
  </section>
  <section id="panel" aria-label="jamparty settings" hidden>
    <div class="control" id="comment-mode-row">
      <span class="label" id="comment-mode-label">Comment mode</span>
      <button class="chip" id="comment-mode-key" type="button" aria-label="Key for comment mode">C</button>
      <button class="switch" id="comment-mode" type="button" role="switch" aria-checked="true" aria-labelledby="comment-mode-label"></button>
    </div>
    <div class="control" id="show-cursor-row">
      <span class="label" id="show-cursor-label">Show my cursor</span>
      <button class="chip" id="show-cursor-key" type="button" aria-label="Key for show my cursor">X</button>
      <button class="switch" id="show-cursor" type="button" role="switch" aria-checked="false" aria-labelledby="show-cursor-label"></button>
    </div>
    <div class="control">
      <span class="label">Clear answers</span>
      <button class="chip" id="clear-answers-key" type="button" aria-label="Key for clear answers">${DEFAULT_CLEAR_ANSWERS_KEY.toUpperCase()}</button>
      <button id="clear-answers" type="button">Clear</button>
    </div>
    <div class="control">
      <label class="label" for="answer-timeout">Answer timeout (s; 0 = off)</label>
      <input id="answer-timeout" type="number" required min="0" max="${MAX_ANSWER_TIMEOUT}" step="1" value="${DEFAULT_ANSWER_TIMEOUT}">
    </div>
    <div class="control" id="corner-row">
      <span class="label" id="corner-label">Corner</span>
      <span id="corners" class="segments" role="group" aria-labelledby="corner-label">
        <button class="corner segment" type="button" data-corner="top-left" aria-label="Top left" aria-pressed="false">↖</button>
        <button class="corner segment" type="button" data-corner="top-right" aria-label="Top right" aria-pressed="false">↗</button>
        <button class="corner segment" type="button" data-corner="bottom-left" aria-label="Bottom left" aria-pressed="false">↙</button>
        <button class="corner segment" type="button" data-corner="bottom-right" aria-label="Bottom right" aria-pressed="true">↘</button>
      </span>
    </div>
    <div class="control">
      <span class="label" id="theme-label">Theme</span>
      <span class="segments" role="group" aria-labelledby="theme-label">
        <button class="segment" type="button" data-theme="auto" aria-pressed="true">Auto</button>
        <button class="segment" type="button" data-theme="dark" aria-pressed="false">Dark</button>
        <button class="segment" type="button" data-theme="light" aria-pressed="false">Light</button>
      </span>
    </div>
    <div class="control" id="text-size-row">
      <span class="label" id="text-size-label">Text size</span>
      <span class="segments" role="group" aria-labelledby="text-size-label">
        <button class="segment" type="button" data-text-size="small" aria-pressed="false">Small</button>
        <button class="segment" type="button" data-text-size="default" aria-pressed="true">Default</button>
        <button class="segment" type="button" data-text-size="large" aria-pressed="false">Large</button>
        <button class="segment" type="button" data-text-size="larger" aria-pressed="false">Larger</button>
      </span>
    </div>
    <div id="name-row">
      <label for="name-field">Name</label>
      <input id="name-field" type="text" maxlength="${MAX_NAME}" autocomplete="nickname" spellcheck="false">
    </div>
    <div id="panel-message" role="status"></div>
    <div id="people">
      <span id="here-count">0 here</span>
      <ul id="avatars" aria-label="People here"></ul>
    </div>
  </section>
  <div id="toast" role="status" aria-live="polite" hidden></div>
  <div id="agent-log" role="log" aria-live="polite" aria-relevant="additions">
    <div id="agent-general"></div>
    <div id="agent-anchored"></div>
  </div>
  <dialog id="jamparty-dialog" aria-labelledby="dialog-label">
    <form>
      <header id="dialog-header">
        <label id="dialog-label" for="comment">Comment:</label>
      </header>
      <div id="selection-preview" hidden>
        <div id="selection-label">Selected text</div>
        <blockquote id="selection-text"></blockquote>
      </div>
      <textarea id="comment" name="comment" autofocus maxlength="${MAX_COMMENT}" required></textarea>
      <ul id="attachments" aria-label="Attachments" hidden></ul>
      <input id="attachment-input" type="file" multiple hidden>
      <div id="missing-attachments" hidden>
        Files from before the reload need to be attached again.
        <button id="discard-missing" type="button">Continue without missing files</button>
      </div>
      <div id="status" role="alert"></div>
      <div id="reload-notice" role="status" hidden>
        Files changed. Reloading when this edit closes.
      </div>
      <div class="actions">
        <button id="attach" type="button" aria-label="Attach files" title="Attach files">${PAPERCLIP_ICON}</button>
        <button id="cancel" type="button">Cancel</button>
        <button id="send" type="submit">Queue comment</button>
      </div>
    </form>
  </dialog>
  <dialog id="name-dialog" aria-labelledby="name-heading">
    <form id="name-form">
      <div id="name-title">
        <strong id="name-heading">Join the Jam.</strong>
        <span>Your name is shown next to your comments and your cursor.</span>
      </div>
      <label for="name-input">Name</label>
      <input id="name-input" type="text" maxlength="${MAX_NAME}" autocomplete="nickname" spellcheck="false" required>
      <div id="name-error" role="alert"></div>
      <div id="name-foot">
        <span>Remembered for this share in this browser; change it under the gear.</span>
        <button id="name-join" type="submit">Join</button>
      </div>
    </form>
  </dialog>
`;
var Ui = class _Ui {
  host;
  shadow;
  cursors;
  highlight;
  editHighlight;
  edit;
  /** The four corner buttons in the control panel. */
  corners;
  messageHighlight;
  bar;
  pill;
  connection;
  badge;
  gear;
  list;
  listHeader;
  listDot;
  listCount;
  connectionText;
  collapse;
  outboxList;
  answerList;
  panel;
  commentModeKey;
  commentMode;
  clearAnswersKey;
  clearAnswers;
  answerTimeout;
  showCursorKey;
  showCursor;
  nameField;
  panelMessage;
  hereCount;
  avatars;
  toast;
  agentGeneral;
  agentAnchored;
  dialog;
  dialogHeader;
  dialogLabel;
  form;
  selectionPreview;
  selectionText;
  textarea;
  attachments;
  attachmentInput;
  attach;
  missingAttachments;
  discardMissing;
  status;
  reloadNotice;
  cancel;
  send;
  nameDialog;
  nameForm;
  nameInput;
  nameError;
  nameJoin;
  root;
  scale = TEXT_SCALES.default;
  get textScale() {
    return this.scale;
  }
  setTextSize(size) {
    this.scale = TEXT_SCALES[size];
    this.root.style.setProperty("--jp-unit", `${this.scale}px`);
  }
  toastTimer;
  /** Adds the UI to the page; null when it is there already. */
  static install() {
    if (document.getElementById(HOST_ID)) return null;
    const host = document.createElement("div");
    host.id = HOST_ID;
    const shadow = host.attachShadow({ mode: "open" });
    shadow.innerHTML = `<style>${styles}</style><div id="overlay-root">${MARKUP}</div>`;
    document.body.append(host);
    return new _Ui(host, shadow);
  }
  constructor(host, shadow) {
    const find = (selector, type) => {
      const element = shadow.querySelector(selector);
      if (!(element instanceof type)) throw new Error(`jamparty: missing ${selector}`);
      return element;
    };
    this.host = host;
    this.shadow = shadow;
    this.root = find("#overlay-root", HTMLDivElement);
    this.setTextSize("default");
    this.cursors = find("#cursors", HTMLDivElement);
    this.highlight = find("#highlight", HTMLDivElement);
    this.editHighlight = find("#edit-highlight", HTMLDivElement);
    this.edit = find("#edit", HTMLButtonElement);
    this.corners = Array.from(shadow.querySelectorAll(".corner")).filter(
      (button) => button instanceof HTMLButtonElement
    );
    this.messageHighlight = find("#message-highlight", HTMLDivElement);
    this.bar = find("#bar", HTMLDivElement);
    this.pill = find("#pill", HTMLButtonElement);
    this.connection = find("#connection", HTMLSpanElement);
    this.badge = find("#badge", HTMLSpanElement);
    this.gear = find("#gear", HTMLButtonElement);
    this.list = find("#list", HTMLElement);
    this.listHeader = find("#list-header", HTMLElement);
    this.listDot = find("#list-dot", HTMLSpanElement);
    this.listCount = find("#list-count", HTMLSpanElement);
    this.connectionText = find("#connection-text", HTMLSpanElement);
    this.collapse = find("#collapse", HTMLButtonElement);
    this.outboxList = find("#outbox-list", HTMLOListElement);
    this.answerList = find("#answer-list", HTMLUListElement);
    this.panel = find("#panel", HTMLElement);
    this.commentModeKey = find("#comment-mode-key", HTMLButtonElement);
    this.commentMode = find("#comment-mode", HTMLButtonElement);
    this.clearAnswersKey = find("#clear-answers-key", HTMLButtonElement);
    this.clearAnswers = find("#clear-answers", HTMLButtonElement);
    this.answerTimeout = find("#answer-timeout", HTMLInputElement);
    this.showCursorKey = find("#show-cursor-key", HTMLButtonElement);
    this.showCursor = find("#show-cursor", HTMLButtonElement);
    this.nameField = find("#name-field", HTMLInputElement);
    this.panelMessage = find("#panel-message", HTMLDivElement);
    this.hereCount = find("#here-count", HTMLSpanElement);
    this.avatars = find("#avatars", HTMLUListElement);
    this.toast = find("#toast", HTMLDivElement);
    this.agentGeneral = find("#agent-general", HTMLDivElement);
    this.agentAnchored = find("#agent-anchored", HTMLDivElement);
    this.dialog = find("#jamparty-dialog", HTMLDialogElement);
    this.dialogHeader = find("#dialog-header", HTMLElement);
    this.dialogLabel = find("#dialog-label", HTMLLabelElement);
    this.form = find("#jamparty-dialog form", HTMLFormElement);
    this.selectionPreview = find("#selection-preview", HTMLDivElement);
    this.selectionText = find("#selection-text", HTMLQuoteElement);
    this.textarea = find("textarea", HTMLTextAreaElement);
    this.attachments = find("#attachments", HTMLUListElement);
    this.attachmentInput = find("#attachment-input", HTMLInputElement);
    this.attach = find("#attach", HTMLButtonElement);
    this.missingAttachments = find("#missing-attachments", HTMLDivElement);
    this.discardMissing = find("#discard-missing", HTMLButtonElement);
    this.status = find("#status", HTMLDivElement);
    this.reloadNotice = find("#reload-notice", HTMLDivElement);
    this.cancel = find("#cancel", HTMLButtonElement);
    this.send = find("#send", HTMLButtonElement);
    this.nameDialog = find("#name-dialog", HTMLDialogElement);
    this.nameForm = find("#name-form", HTMLFormElement);
    this.nameInput = find("#name-input", HTMLInputElement);
    this.nameError = find("#name-error", HTMLDivElement);
    this.nameJoin = find("#name-join", HTMLButtonElement);
  }
  showToast(message) {
    this.toast.textContent = message;
    this.toast.hidden = false;
    if (this.toastTimer !== void 0) clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => {
      this.toast.hidden = true;
    }, 2400);
  }
  setPending(pending, sending, refused) {
    const parts = [`${pending} queued`];
    if (sending > 0) parts.push(`${sending} sending`);
    if (refused > 0) parts.push(`${refused} refused`);
    const text = parts.join(" · ");
    if (this.badge.textContent !== text) this.badge.textContent = text;
    this.badge.dataset.pending = String(pending);
    this.listCount.textContent = text;
  }
};

// client/src/main.ts
function install(pdf) {
  const ui = Ui.install();
  if (ui === null) return;
  let url = pageUrl();
  let you = null;
  let here = [];
  let pending = 0;
  let shownRevision = LIVE_REVISION;
  const socket = new PageSocket({
    hello(hello) {
      applyHello(hello);
    },
    slice(slice) {
      applySlice(slice);
    },
    open(open) {
      connection.setOpen(open);
      if (open) report();
      else sender.setPlaced(false);
    }
  });
  const outbox = new Outbox(async (submission) => {
    const outcome = await socket.comment(submission);
    if (submission.choice !== void 0 && "refused" in outcome && outcome.refused.choiceRefused === true) {
      answers.remove(submission.selector.slice(AGENT_TARGET_PREFIX.length));
    }
    return outcome;
  }, () => connection.isReachable);
  const changeSettings = async (change) => {
    const before = remembered();
    remember(change);
    const outcome = await socket.changeSettings(change);
    if (outcome !== "saved" && socket.isOpen) keep(before);
    return outcome;
  };
  const answers = new Answers(ui, ({ ref, ...draft }) => outbox.submit(draft, ref), pdf);
  const dialog = new CommentDialog(ui, outbox);
  const list = new CommentList(ui, outbox, (answer, node) => {
    answers.renderHistory(answer, node);
  });
  answers.onHistory = (history2) => {
    list.setAnswers(history2);
  };
  answers.historyTarget = (id) => list.answerTarget(id);
  const panel = new Panel(ui, changeSettings);
  panel.onClearAnswers = () => {
    answers.clear();
  };
  const sender = new CursorSender(
    ui,
    (position) => socket.send({ type: "cursor", position })
  );
  const layer = new CursorLayer(ui);
  const editMode = pdf === void 0 ? new EditMode(ui, dialog) : null;
  pdf?.bind(ui, dialog, () => {
    panel.close();
    list.close();
  });
  if (editMode !== null) {
    dialog.onOpen = () => {
      editMode.clearHover();
    };
    editMode.onPagePress = () => {
      panel.close();
      list.close();
    };
  }
  list.onPreview = (entry) => {
    pdf?.previewComment(entry);
  };
  list.onEdit = async (entry) => {
    if (!isThisPage(entry.draft.page)) {
      try {
        const target = new URL(entry.draft.page);
        if (target.origin === location.origin) {
          if (target.pathname !== location.pathname || target.search !== location.search) {
            sessionStorage.setItem(EDIT_REF_KEY, entry.ref);
            location.assign(target);
            return;
          }
          location.assign(target);
          await new Promise((resolve) => {
            window.addEventListener("hashchange", () => {
              requestAnimationFrame(() => {
                resolve();
              });
            }, { once: true });
          });
          await targetMounted(entry.draft.selector, HASH_ROUTE_WAIT_MS);
        }
      } catch {
      }
    }
    if (await dialog.edit(entry, (selector) => isThisPage(entry.draft.page) ? answers.targetFor(selector) : null)) {
      list.close();
    }
  };
  list.onOpen = () => {
    panel.close();
    pdf?.closeOutline();
  };
  panel.onOpen = () => {
    list.close();
    pdf?.closeOutline();
  };
  outbox.onChange = () => {
    list.render();
    pdf?.showComments(outbox.entries);
    const sending = outbox.entries.filter(
      (entry) => entry.draft.choice === void 0 && outbox.shown(entry) === "sending"
    ).length;
    const refused = outbox.entries.filter((entry) => entry.state === "failed").length;
    ui.setPending(pending, sending, refused);
  };
  outbox.onInitialOutcome = (outcome) => {
    if ("commented" in outcome) {
      ui.showToast(`Comment queued · ${pending} pending`);
    } else if ("unsent" in outcome) {
      ui.showToast("Not sent: kept in the page, sent when the connection is back");
    }
  };
  const prompt = new NamePrompt(ui, changeSettings);
  panel.onSettings = (settings) => {
    answers.setTimeoutSeconds(settings.answerTimeout);
    answers.textSizeChanged();
    sender.setOn(settings.showCursor === "on");
    prompt.show(settings);
  };
  const connection = new Connection((view, reached) => {
    list.showState(view);
    if (view.colour !== "green") layer.clear();
    if (!connection.isReachable) pdf?.suspend();
    if (reached) {
      void outbox.resendAll();
      pdf?.ensureLoaded();
    }
  });
  const showPeople = (people) => {
    here = people;
    panel.showPeople(here, you);
    layer.setPeople(here, url);
  };
  const showMine = (undelivered) => {
    if (undelivered === null) return;
    pending = undelivered.length;
    outbox.sync(undelivered);
  };
  const onRevision = (revision) => {
    if (revision === null) return;
    if (pdf !== void 0) {
      pdf.requestRevision(revision);
      if (connection.isReachable) pdf.ensureLoaded();
      return;
    }
    if (shownRevision === null || revision <= shownRevision) return;
    shownRevision = revision;
    dialog.reloadWhenClosed();
  };
  let restored = false;
  let editRef = null;
  try {
    editRef = sessionStorage.getItem(EDIT_REF_KEY);
    sessionStorage.removeItem(EDIT_REF_KEY);
  } catch {
  }
  const restoreDraft = () => {
    if (restored) return;
    restored = true;
    const edit = outbox.entries.find((entry) => entry.ref === editRef && entry.state === "failed" && isThisPage(entry.draft.page));
    if (edit !== void 0) {
      void list.onEdit(edit);
      return;
    }
    void dialog.restore((selector) => answers.targetFor(selector));
  };
  function applyHello(hello) {
    you = hello.link.you?.key ?? null;
    outbox.setKey(you);
    showMine(hello.mine);
    answers.sync(hello.answers);
    restoreDraft();
    showSettings(hello.settings);
    showPeople(hello.people);
    onRevision(hello.revision);
    connection.setLink(hello.link);
  }
  function showSettings(settings) {
    const change = missing(remembered(), settings);
    if (change === null) {
      panel.show(settings);
      return;
    }
    panel.show({ ...settings, askName: settings.askName && change.name === void 0 });
    void socket.changeSettings(change).then((outcome) => {
      if (outcome === "saved" || !socket.isOpen) return;
      forget();
      panel.show(settings);
    });
  }
  function applySlice(slice) {
    switch (slice.type) {
      case "link":
        you = slice.you?.key ?? null;
        outbox.setKey(you);
        showPeople(here);
        connection.setLink(slice);
        break;
      case "people":
        showPeople(slice.here);
        break;
      case "mine":
        showMine(slice.undelivered);
        break;
      case "settings":
        panel.show(slice);
        break;
      case "reload":
        onRevision(slice.revision);
        break;
      case "answer_resolved":
        answers.remove(slice.id);
        break;
      case "answer":
        answers.receive(slice);
        break;
      case "cursor":
        if (slice.url === url && slice.key !== you) layer.update(slice.key, slice.position);
        break;
    }
  }
  function report() {
    if (!socket.send({ type: "page", url })) return;
    sender.setPlaced(url !== null);
  }
  watchUrl((now) => {
    answers.schedulePlacement();
    url = now;
    layer.clear();
    layer.setPeople(here, url);
    report();
  });
  const activate = () => {
    if (document.visibilityState === "visible") report();
  };
  window.addEventListener("focus", activate);
  document.addEventListener("visibilitychange", activate);
  const commenting = () => panel.commentMode;
  document.addEventListener("contextmenu", (event) => {
    if (!commenting()) return;
    if (event.shiftKey || event.composedPath().includes(ui.host)) return;
    if (!(event.target instanceof Element)) return;
    if (pdf !== void 0) {
      if (pdf.commentAtPointer(event)) event.preventDefault();
      return;
    }
    event.preventDefault();
    dialog.openElement(event.target, event.clientX, event.clientY);
  }, true);
  ui.shadow.addEventListener("contextmenu", (event) => {
    if (!commenting()) return;
    if (!(event instanceof MouseEvent) || event.shiftKey) return;
    const target = event.composedPath().find(
      (node) => node instanceof HTMLElement && node.matches(".agent-message, .agent-reply")
    );
    if (!(target instanceof HTMLElement) || !target.dataset.messageId) return;
    event.preventDefault();
    event.stopPropagation();
    dialog.openElement(
      target,
      event.clientX,
      event.clientY,
      `${AGENT_TARGET_PREFIX}${target.dataset.messageId}`
    );
  }, true);
  document.addEventListener("keydown", (event) => {
    if (!commenting()) return;
    if (event.defaultPrevented || event.isComposing || event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) return;
    if (isEditing(event)) return;
    event.preventDefault();
    if (!pdf?.commentOnSelection()) dialog.openPage();
  }, true);
  outbox.onChange();
  if (editRef === null && loadDraft()?.target.kind !== "answer") restoreDraft();
  socket.start();
}
var HASH_ROUTE_WAIT_MS = 2e3;
function targetMounted(selector, timeout) {
  const found = () => {
    try {
      return selector !== "" && document.querySelector(selector) !== null;
    } catch {
      return false;
    }
  };
  if (selector === "" || selector.startsWith(AGENT_TARGET_PREFIX) || found()) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    const done = () => {
      observer.disconnect();
      clearTimeout(timer);
      resolve();
    };
    const observer = new MutationObserver(() => {
      if (found()) done();
    });
    observer.observe(document.documentElement, {
      subtree: true,
      childList: true,
      attributes: true
    });
    const timer = window.setTimeout(done, timeout);
  });
}

// client/src/index.ts
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", () => {
    install();
  }, { once: true });
} else {
  install();
}
