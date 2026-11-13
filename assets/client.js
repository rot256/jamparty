// Generated from client/src by `npm run build`; do not edit.

// client/src/generated/api.ts
var MAX_NAME = 40;
var MAX_COMMENT = 1e4;
var MAX_SELECTED_TEXT = 2e3;
var MAX_PAGE_PATH = 512;
var MAX_CURSOR_SELECTOR = 512;
var FRACTION_ONE = 1e4;
var RESERVED_KEYS = ["/", "Escape"];

// client/src/capture.ts
function truncate(value, length) {
  return value.length <= length ? value : `${value.slice(0, length - 1)}…`;
}
function normalizeText(value) {
  return value.replace(/\s+/g, " ").trim();
}
function selectedTextFor(target) {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return null;
  try {
    if (!selection.getRangeAt(0).intersectsNode(target)) return null;
  } catch {
    return null;
  }
  const selected = normalizeText(selection.toString());
  return selected ? truncate(selected, MAX_SELECTED_TEXT) : null;
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
var HOST_ID = "jampot";
var DISMISSED_KEY = "jampot-dismissed-v1";
var OUTBOX_KEY = "jampot-outbox-v1";
var DRAFT_KEY = "jampot-draft-v1";
var MAX_OUTBOX = 20;
var WATCHDOG_MS = 35e3;
var RECONNECT_MIN_MS = 250;
var RECONNECT_MAX_MS = 5e3;
var AGENT_TARGET_PREFIX = "jampot-agent:";
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
var GAP = 8;
var EDGE = 8;
function loadDismissed() {
  try {
    const stored = JSON.parse(sessionStorage.getItem(DISMISSED_KEY) ?? "[]");
    return new Set(
      Array.isArray(stored) ? stored.filter((id) => typeof id === "string") : []
    );
  } catch {
    return /* @__PURE__ */ new Set();
  }
}
var Answers = class {
  constructor(ui) {
    this.ui = ui;
    const schedule = () => {
      this.schedulePlacement();
    };
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    new MutationObserver(schedule).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true
    });
  }
  ui;
  shown = /* @__PURE__ */ new Map();
  dismissed = loadDismissed();
  placementFrame;
  highlightTimer;
  /** The node of answer `id`, for comments on an answer. */
  node(id) {
    return this.shown.get(id)?.node ?? null;
  }
  /** The element a selector names: on the page, or one of the answers. */
  targetFor(selector) {
    if (selector.startsWith(AGENT_TARGET_PREFIX)) {
      return this.node(selector.slice(AGENT_TARGET_PREFIX.length));
    }
    return document.querySelector(selector);
  }
  add(answer) {
    if (this.dismissed.has(answer.id) || this.shown.has(answer.id)) return;
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
    const body = document.createElement("div");
    body.className = "agent-body";
    body.innerHTML = answer.html;
    const unavailable = document.createElement("div");
    unavailable.className = "agent-unavailable";
    unavailable.textContent = "Target unavailable";
    unavailable.hidden = true;
    node.append(close, to, body, unavailable);
    const anchor = answer.anchor !== void 0 && samePage(answer.page) ? answer.anchor : null;
    if (anchor !== null) {
      node.classList.add("has-anchor");
      node.tabIndex = 0;
      node.setAttribute("aria-label", "Agent message; activate to show its anchor");
      node.addEventListener("click", (event) => {
        const interactive = event.target instanceof Element ? event.target.closest("a, button") : null;
        const selection = window.getSelection();
        if (interactive || selection && !selection.isCollapsed) return;
        this.reveal(anchor);
      });
      node.addEventListener("keydown", (event) => {
        if (event.target !== node || !["Enter", " "].includes(event.key)) return;
        event.preventDefault();
        this.reveal(anchor);
      });
    }
    for (const link of body.querySelectorAll("a")) {
      const href = link.getAttribute("href") ?? "";
      if (!href.toLowerCase().startsWith("jampot:")) {
        link.target = "_blank";
        link.rel = "noopener noreferrer";
      }
    }
    body.addEventListener("click", (event) => {
      const link = event.target instanceof Element ? event.target.closest("a") : null;
      if (link) this.followElementLink(link, event);
    });
    close.addEventListener("click", () => {
      this.dismissed.add(answer.id);
      this.persistDismissed();
      this.shown.delete(answer.id);
      node.remove();
      this.schedulePlacement();
    });
    this.shown.set(answer.id, { node, unavailable, anchor });
    this.schedulePlacement();
  }
  persistDismissed() {
    try {
      sessionStorage.setItem(DISMISSED_KEY, JSON.stringify(Array.from(this.dismissed)));
    } catch {
    }
  }
  /** The element an anchored answer belongs next to; undefined when it is not shown. */
  anchorTarget(anchor) {
    try {
      const target = this.targetFor(anchor);
      if (!target || target === this.ui.host || target.getClientRects().length === 0) {
        return void 0;
      }
      return target;
    } catch {
      return void 0;
    }
  }
  schedulePlacement() {
    if (this.placementFrame !== void 0) return;
    this.placementFrame = requestAnimationFrame(() => {
      this.place();
    });
  }
  place() {
    this.placementFrame = void 0;
    const { agentAnchored, agentGeneral } = this.ui;
    const groups = /* @__PURE__ */ new Map();
    for (const entry of this.shown.values()) {
      const target = entry.anchor === null ? void 0 : this.anchorTarget(entry.anchor);
      if (target !== void 0) {
        entry.unavailable.hidden = true;
        if (entry.node.parentElement !== agentAnchored) agentAnchored.append(entry.node);
        const group = groups.get(target) ?? [];
        group.push(entry.node);
        groups.set(target, group);
      } else {
        entry.unavailable.hidden = entry.anchor === null;
        if (entry.node.parentElement !== agentGeneral) agentGeneral.append(entry.node);
        entry.node.style.removeProperty("left");
        entry.node.style.removeProperty("top");
      }
    }
    const occupied = Array.from(agentGeneral.children).map((node) => {
      const rect = node.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
    });
    const overlaps = (candidate) => occupied.some((rect) => !(candidate.right + GAP <= rect.left || candidate.left >= rect.right + GAP || candidate.bottom + GAP <= rect.top || candidate.top >= rect.bottom + GAP));
    const placement = (rect, width, height, desiredTop) => {
      const maximumLeft = window.innerWidth - width - EDGE;
      const right = Math.max(EDGE, Math.min(rect.right + GAP, maximumLeft));
      const left = Math.max(EDGE, Math.min(rect.left - width - GAP, maximumLeft));
      const horizontal = rect.right + GAP + width <= window.innerWidth - EDGE ? [right, left] : [left, right];
      const maximumTop = window.innerHeight - height - EDGE;
      const vertical = [
        desiredTop,
        ...occupied.flatMap((placed) => [placed.bottom + GAP, placed.top - height - GAP])
      ].map((top) => Math.max(EDGE, Math.min(top, maximumTop)));
      const candidates = horizontal.flatMap((candidateLeft, side) => vertical.map((candidateTop) => ({
        left: candidateLeft,
        right: candidateLeft + width,
        top: candidateTop,
        bottom: candidateTop + height,
        score: Math.abs(candidateTop - desiredTop) * 2 + side
      }))).sort((first, second) => first.score - second.score);
      const fallback = { left: EDGE, right: EDGE + width, top: desiredTop, bottom: desiredTop + height };
      return candidates.find((candidate) => !overlaps(candidate)) ?? candidates[0] ?? fallback;
    };
    for (const [target, nodes] of groups) {
      const rect = target.getBoundingClientRect();
      const heights = nodes.map((node) => node.offsetHeight);
      const totalHeight = heights.reduce((total, height) => total + height, 0) + GAP * Math.max(0, nodes.length - 1);
      let top = Math.max(EDGE, Math.min(rect.top, window.innerHeight - totalHeight - EDGE));
      for (const [index, node] of nodes.entries()) {
        const placed = placement(rect, node.offsetWidth, heights[index] ?? 0, top);
        node.style.left = `${placed.left}px`;
        node.style.top = `${placed.top}px`;
        occupied.push(placed);
        top = placed.bottom + GAP;
      }
    }
  }
  showHighlight(target) {
    const { messageHighlight } = this.ui;
    const rect = target.getBoundingClientRect();
    messageHighlight.style.display = "block";
    messageHighlight.style.left = `${rect.left}px`;
    messageHighlight.style.top = `${rect.top}px`;
    messageHighlight.style.width = `${rect.width}px`;
    messageHighlight.style.height = `${rect.height}px`;
    if (this.highlightTimer !== void 0) clearTimeout(this.highlightTimer);
    this.highlightTimer = window.setTimeout(() => {
      messageHighlight.style.display = "none";
    }, 1800);
  }
  /** Scrolls to the element `selector` names and highlights it briefly. */
  reveal(selector) {
    let target;
    try {
      target = this.targetFor(selector);
    } catch {
      target = null;
    }
    if (!target || target.getClientRects().length === 0) {
      this.ui.showToast("Element is unavailable");
      return;
    }
    const found = target;
    found.scrollIntoView({ block: "center", inline: "nearest" });
    requestAnimationFrame(() => {
      this.showHighlight(found);
    });
  }
  followElementLink(link, event) {
    const href = link.getAttribute("href") ?? "";
    if (!href.toLowerCase().startsWith("jampot:")) return;
    event.preventDefault();
    let selector;
    try {
      selector = decodeURIComponent(href.slice("jampot:".length));
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
  get view() {
    let colour = "green";
    let text = "connected";
    if (!this.socketOpen) {
      colour = "amber";
      text = "reconnecting to jampot join…";
    } else if (this.link?.connected !== true) {
      colour = "grey";
      text = "share unreachable";
    } else if (this.link.status !== null) {
      colour = "grey";
      text = this.link.status;
    }
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

// client/src/panel.ts
var CORNERS = ["bottom-right", "bottom-left", "top-right", "top-left"];
function isCorner(value) {
  return CORNERS.some((corner) => corner === value);
}
var ACTION_NAMES = {
  commentMode: "comment mode",
  showCursor: "show my cursor"
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
  if (outcome === "saved" || "commented" in outcome) return null;
  return "unsent" in outcome ? outcome.unsent.error : outcome.refused.error;
}
function colourOf(person) {
  return PALETTE[person.colour % PALETTE.length] ?? "#555";
}
var Panel = class {
  constructor(ui, changeSettings) {
    this.ui = ui;
    this.changeSettings = changeSettings;
    ui.gear.addEventListener("click", () => {
      this.toggle();
    });
    ui.commentMode.addEventListener("click", () => {
      this.flip("commentMode");
    });
    ui.showCursor.addEventListener("click", () => {
      this.flip("showCursor");
    });
    for (const button of ui.corners) {
      button.addEventListener("click", () => {
        const corner = button.dataset.corner;
        if (!isCorner(corner) || this.settings?.corner === corner) return;
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
  settings = null;
  listening = null;
  /** Called when the panel opens (the comment list closes then). */
  onOpen = () => void 0;
  /** Called with the settings whenever they change. */
  onSettings = () => void 0;
  chips() {
    return [["commentMode", this.ui.commentModeKey], ["showCursor", this.ui.showCursorKey]];
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
    ui.showCursor.setAttribute("aria-checked", String(settings.showCursor === "on"));
    for (const [action, chip] of this.chips()) {
      if (this.listening !== action) chip.textContent = keyLabel(settings.keys[action]);
    }
    if (ui.shadow.activeElement !== ui.nameField) ui.nameField.value = settings.name;
    for (const button of ui.corners) {
      button.setAttribute("aria-pressed", String(button.dataset.corner === settings.corner));
    }
    ui.host.dataset.corner = settings.corner;
    this.onSettings(settings);
  }
  /**
   * Asks the joiner for a change. What it saved comes as a settings
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
    const chip = action === "commentMode" ? this.ui.commentModeKey : this.ui.showCursorKey;
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
    if (["Shift", "Control", "Alt", "Meta", "CapsLock"].includes(event.key)) return;
    const key = normalizeKey(event.key);
    const other = action === "commentMode" ? "showCursor" : "commentMode";
    if (RESERVED_KEYS.includes(key) || key === " ") {
      this.stopListening();
      this.message(`${keyLabel(key)} is used by jampot`);
      return;
    }
    if (normalizeKey(settings.keys[other]) === key) {
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
    for (const action of ["commentMode", "showCursor"]) {
      if (normalizeKey(settings.keys[action]) === key) {
        event.preventDefault();
        event.stopImmediatePropagation();
        this.flip(action);
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
    if (position === null) return;
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
  /** The element under the point (not jampot's own overlay) and the offset in its box. */
  position(x, y) {
    const element = document.elementsFromPoint(x, y).find((candidate) => candidate !== this.ui.host && !this.ui.host.contains(candidate));
    if (element === void 0) return null;
    const selector = cssSelector(element);
    if (selector.length > MAX_CURSOR_SELECTOR) return null;
    const rect = element.getBoundingClientRect();
    return {
      selector,
      x: fraction(x - rect.left, rect.width),
      y: fraction(y - rect.top, rect.height)
    };
  }
};
var SVG_NS = "http://www.w3.org/2000/svg";
var CursorLayer = class {
  constructor(ui) {
    this.ui = ui;
  }
  ui;
  drawn = /* @__PURE__ */ new Map();
  people = /* @__PURE__ */ new Map();
  frame;
  /** Someone's pointer on this page moved, or (null) left it. */
  update(key, position) {
    if (position === null) {
      this.remove(key);
      return;
    }
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
    for (const key of [...this.drawn.keys()]) {
      const person = this.people.get(key);
      if (person?.cursor !== "on" || url === null || person.url !== url) {
        this.remove(key);
      }
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
  /** Glides each cursor towards its point (kept on its element as the page scrolls) and fades idle ones. */
  animate() {
    this.frame = void 0;
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
      if (!idle) moving = true;
    }
    if (moving) this.schedule();
  }
};

// client/src/json.ts
function isRecord(value) {
  return typeof value === "object" && value !== null;
}

// client/src/draft.ts
function parseKind(value) {
  switch (value) {
    case "element":
    case "page":
    case "answer":
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
function parse(value) {
  if (!isRecord(value)) return null;
  const { page, unavailable, selectedText, comment } = value;
  const target = parseTarget(value.target);
  if (typeof page !== "string" || typeof comment !== "string" || target === null) return null;
  return {
    page,
    target,
    unavailable: unavailable === true,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment
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
function isThisPage(page) {
  try {
    const url = new URL(page);
    const here = window.location;
    return url.pathname === here.pathname && url.search === here.search && url.hash === here.hash;
  } catch {
    return false;
  }
}
function loadDraft() {
  try {
    const draft = parse(JSON.parse(sessionStorage.getItem(DRAFT_KEY) ?? "null"));
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
      return ELEMENT_LABEL;
    case "answer":
      return ANSWER_LABEL;
  }
}
var CommentDialog = class {
  constructor(ui, outbox, pending) {
    this.ui = ui;
    this.outbox = outbox;
    this.pending = pending;
    const { dialog, form, textarea, cancel } = ui;
    form.addEventListener("submit", (event) => {
      event.preventDefault();
      this.submit();
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
    cancel.addEventListener("click", () => {
      this.dismiss();
    });
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      this.dismiss();
    });
    dialog.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const rect = dialog.getBoundingClientRect();
      if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) this.dismiss();
    });
    dialog.addEventListener("close", () => {
      this.stopTracking();
      ui.highlight.style.display = "none";
    });
  }
  ui;
  outbox;
  pending;
  captured = null;
  /** What the dialog was opened on, which a draft keeps. */
  target = { kind: "page", selector: "" };
  /** The target was gone after a reload: a page comment now. */
  unavailable = false;
  /** Handed to the outbox and on its way: no draft any more. */
  sending = false;
  /** A live reload waiting for the dialog to close. */
  pendingReload = false;
  get open() {
    return this.ui.dialog.open;
  }
  /**
   * Closes the dialog at the person's request (Cancel, Escape, a click
   * outside), unless its comment is on its way: it stays until the joiner
   * answers, so no other comment starts meanwhile.
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
  restore(find) {
    const draft = loadDraft();
    if (draft === null || this.ui.dialog.open) return;
    const { target, selectedText } = draft;
    let element = null;
    if (target.kind !== "page" && !draft.unavailable) {
      try {
        element = find(target.selector);
      } catch {
      }
    }
    const page = window.location.href;
    let x = window.innerWidth / 2;
    let y = window.innerHeight / 3;
    let captured = { element: null, page, selector: "", selectedText };
    if (element !== null) {
      captured = { element, page, selector: target.selector, selectedText };
      const rect = element.getBoundingClientRect();
      x = rect.left;
      y = rect.bottom;
    }
    this.show(captured, x, y, target);
    this.unavailable = target.kind !== "page" && element === null;
    this.ui.textarea.value = draft.comment;
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
      comment: this.ui.textarea.value
    });
  }
  updateHighlight = () => {
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
  stopTracking() {
    window.removeEventListener("resize", this.updateHighlight);
    window.removeEventListener("scroll", this.updateHighlight, true);
  }
  place(x, y) {
    const { dialog } = this.ui;
    const gap = 12;
    const width = dialog.offsetWidth;
    const height = dialog.offsetHeight;
    dialog.style.left = `${Math.max(gap, Math.min(x + gap, window.innerWidth - width - gap))}px`;
    dialog.style.top = `${Math.max(gap, Math.min(y + gap, window.innerHeight - height - gap))}px`;
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
    ui.dialog.showModal();
    this.place(x, y);
    this.updateHighlight();
    window.addEventListener("resize", this.updateHighlight);
    window.addEventListener("scroll", this.updateHighlight, true);
    ui.textarea.focus();
    this.keep();
  }
  close() {
    const ui = this.ui;
    ui.textarea.blur();
    if (ui.dialog.open) ui.dialog.close();
    this.stopTracking();
    ui.highlight.style.display = "none";
    this.captured = null;
    this.unavailable = false;
    clearDraft();
    ui.selectionPreview.hidden = true;
    ui.selectionText.textContent = "";
    ui.status.textContent = "";
    ui.textarea.value = "";
    if (this.pendingReload) window.location.reload();
  }
  submit() {
    const ui = this.ui;
    const captured = this.captured;
    if (!captured || this.sending) return;
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
      comment
    };
    const ref = crypto.randomUUID();
    this.sending = true;
    clearDraft();
    void this.outbox.submit(draft, ref).then((outcome) => {
      this.sending = false;
      ui.send.disabled = false;
      ui.cancel.disabled = false;
      if (outcome === "saved" || "commented" in outcome) {
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
    });
  }
};

// client/src/edit.ts
var PRESS_EVENTS = ["pointerdown", "mousedown", "pointerup", "mouseup", "dblclick"];
var EditMode = class {
  constructor(ui, dialog, allowed) {
    this.ui = ui;
    this.dialog = dialog;
    this.allowed = allowed;
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
  allowed;
  on = false;
  hovered = null;
  /**
   * Called on each press edit mode takes from the page: the document's own
   * listeners (closing the panel and the list on outside presses) do not
   * see it.
   */
  onPagePress = () => void 0;
  /** Follows whether edit mode may be on; leaves it when it may not. */
  refresh() {
    const allowed = this.allowed();
    this.ui.edit.disabled = !allowed;
    if (!allowed && this.on) this.leave();
  }
  enter() {
    if (!this.allowed()) return;
    this.on = true;
    this.ui.edit.setAttribute("aria-pressed", "true");
  }
  leave() {
    this.on = false;
    this.ui.edit.setAttribute("aria-pressed", "false");
    this.outline(null);
  }
  /** The page element an event is about, or null for jampot's own overlay. */
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
function stateText(entry) {
  switch (entry.state) {
    case "unsent":
      return "not sent";
    case "sending":
      return "sending";
    case "queued":
    case "delivered":
      return entry.state;
  }
}
var CommentList = class {
  constructor(ui, outbox) {
    this.ui = ui;
    this.outbox = outbox;
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
  render() {
    const { outboxList } = this.ui;
    const now = Date.now();
    const entries = [...this.outbox.entries].sort((first, second) => second.at - first.at);
    outboxList.replaceChildren(...entries.map((entry) => {
      const item = document.createElement("li");
      item.dataset.state = entry.state;
      const row = document.createElement("div");
      row.className = "row";
      const where = document.createElement("span");
      where.className = "where";
      where.textContent = `${shortSelector(entry.draft.selector)} · ${relativeTime(entry.at, now)}`;
      where.title = entry.draft.selector || "the whole page";
      const state = document.createElement("span");
      state.className = "state";
      state.textContent = stateText(entry);
      row.append(where, state);
      const text = document.createElement("p");
      text.className = "text";
      text.textContent = entry.draft.comment;
      item.append(row, text);
      if (entry.state === "unsent") {
        const again = document.createElement("button");
        again.type = "button";
        again.className = "again";
        again.textContent = "send again";
        again.addEventListener("click", () => {
          void this.outbox.resend(entry);
        });
        item.append(again);
      }
      return item;
    }));
  }
};

// client/src/location.ts
function pageUrl() {
  const url = new URL(window.location.href);
  const raw = `${url.pathname}${url.search}${url.hash}`;
  const encoded = raw.replace(/[^\x21-\x7e]|["\\]/gu, (character) => encodeURIComponent(character));
  return encoded.startsWith("/") && encoded.length <= MAX_PAGE_PATH ? encoded : null;
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

// client/src/outbox.ts
function parseDraft(value) {
  if (!isRecord(value)) return null;
  const { page, selector, selectedText, comment } = value;
  if (typeof page !== "string" || typeof selector !== "string" || typeof comment !== "string") {
    return null;
  }
  return {
    page,
    selector,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment
  };
}
function parseState(value) {
  switch (value) {
    case "queued":
    case "delivered":
      return value;
    default:
      return "unsent";
  }
}
function parseEntry(value) {
  if (!isRecord(value)) return null;
  const draft = parseDraft(value.draft);
  if (draft === null) return null;
  return {
    draft,
    ref: typeof value.ref === "string" ? value.ref : crypto.randomUUID(),
    id: typeof value.id === "string" ? value.id : null,
    state: parseState(value.state),
    at: typeof value.at === "number" ? value.at : Date.now()
  };
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
  constructor(send) {
    this.send = send;
  }
  send;
  entries = load();
  /** Called after every change, to redraw. */
  onChange = () => void 0;
  /**
   * The ids of this joiner's comments the share holds, as the joiner last
   * said; null while it does not know (a restarted joiner before the
   * share admitted it).
   */
  undelivered = null;
  /** Persists the list (bounded) and redraws. */
  changed() {
    while (this.entries.length > MAX_OUTBOX) {
      const delivered = this.entries.findIndex((entry) => entry.state === "delivered");
      this.entries.splice(delivered === -1 ? 0 : delivered, 1);
    }
    try {
      sessionStorage.setItem(OUTBOX_KEY, JSON.stringify(this.entries));
    } catch {
    }
    this.onChange();
  }
  /** A queued entry the share no longer holds was delivered. */
  settle(entry) {
    if (entry.state !== "queued" || entry.id === null || this.undelivered === null) return;
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
  /**
   * Sends `draft` under `ref`, kept first: a reload before the joiner's
   * answer sends it again under the same ref, so it is queued once.
   * Resolves with the outcome once the entry reflects it; a refused
   * comment is taken out again (it goes back to the dialog).
   */
  async submit(draft, ref) {
    const entry = { draft, ref, id: null, state: "sending", at: Date.now() };
    this.entries.push(entry);
    this.changed();
    const outcome = await this.send({ ref, ...draft });
    if (typeof outcome !== "string" && "refused" in outcome) {
      const index = this.entries.indexOf(entry);
      if (index !== -1) this.entries.splice(index, 1);
      this.changed();
    } else {
      this.take(entry, outcome);
    }
    return outcome;
  }
  /** Records what became of `entry`. */
  take(entry, outcome) {
    if (typeof outcome !== "string" && "commented" in outcome) {
      entry.id = outcome.commented.id;
      entry.state = "queued";
      this.settle(entry);
    } else {
      entry.state = "unsent";
    }
    this.changed();
  }
  async resend(entry) {
    if (entry.state !== "unsent") return;
    entry.state = "sending";
    this.changed();
    const outcome = await this.send({ ref: entry.ref, ...entry.draft });
    this.take(entry, outcome);
  }
  async resendAll() {
    for (const entry of this.entries.filter((candidate) => candidate.state === "unsent")) {
      await this.resend(entry);
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
var LOST = { unsent: { error: "jampot join is not reachable" } };
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
  waiting = /* @__PURE__ */ new Map();
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
      case "reply": {
        const settle = this.waiting.get(message.to);
        this.waiting.delete(message.to);
        settle?.(message.outcome);
        break;
      }
      case "ping":
        break;
      case "link":
      case "people":
      case "mine":
      case "settings":
      case "reload":
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
  /** Makes a request; its outcome ("unsent" if the socket goes first). */
  request(request) {
    this.n += 1;
    const n = this.n;
    return new Promise((resolve) => {
      if (!this.send({ ...request, n })) {
        resolve(LOST);
        return;
      }
      this.waiting.set(n, resolve);
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
    for (const settle of this.waiting.values()) settle(LOST);
    this.waiting.clear();
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
var styles = `
  :host {
    all: initial;
    color-scheme: light;
    font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    font-size: 13px;
  }

  *, *::before, *::after { box-sizing: border-box; }

  #highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid #eb5e28;
    background: rgb(235 94 40 / 10%);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 90%);
    pointer-events: none;
  }

  #edit-highlight {
    position: fixed;
    display: none;
    z-index: 2147483645;
    border: 2px solid #2f80ed;
    background: rgb(47 128 237 / 8%);
    pointer-events: none;
  }

  #bar {
    position: fixed;
    right: 14px;
    bottom: 14px;
    z-index: 2147483644;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  #bar[hidden] { display: none; }

  #pill {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px 7px 9px;
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 999px;
    background: #161616;
    color: #fff;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    cursor: pointer;
    font: 600 12px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .dot {
    display: block;
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: #9a958e;
  }

  .dot[data-state="green"] { background: #2f9e44; }
  .dot[data-state="amber"] { background: #e8a33d; }
  .dot[data-state="grey"] { background: #9a958e; }

  #gear, #edit {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    padding: 0;
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 50%;
    background: #161616;
    color: #fff;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    cursor: pointer;
  }

  #gear[aria-expanded="true"] { background: #2a2a2a; }
  #edit[aria-pressed="true"] { border-color: #2f80ed; background: #2f80ed; }
  #edit:disabled { opacity: 0.4; cursor: default; }

  #list {
    position: fixed;
    right: 14px;
    bottom: 14px;
    z-index: 2147483644;
    display: flex;
    flex-direction: column;
    width: min(440px, calc(100vw - 28px));
    max-height: min(60vh, calc(100vh - 28px));
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 12px;
    background: #161616;
    color: #fff;
    box-shadow: 0 8px 28px rgb(0 0 0 / 28%);
    font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    overflow: hidden;
  }

  #list[hidden] { display: none; }

  #list header {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid #2c2c2c;
    cursor: pointer;
  }

  #list-summary {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    font-weight: 600;
  }

  #connection-text {
    overflow: hidden;
    color: #b8b2a9;
    font-weight: 400;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  #connection-text[data-state="grey"], #connection-text[data-state="amber"] { color: #f0c27a; }

  #collapse {
    flex: none;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: #b8b2a9;
    font: 16px/1 ui-monospace, Menlo, monospace;
  }

  #collapse:hover, #collapse:focus-visible { background: #2a2a2a; }

  #outbox-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 4px 0;
    overflow: auto;
    list-style: none;
  }

  #outbox-list li {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 14px;
    border-bottom: 1px solid #242424;
  }

  #outbox-list li:last-child { border-bottom: 0; }
  #outbox-list .row { display: flex; justify-content: space-between; gap: 10px; }
  #outbox-list .where, #outbox-list .state { color: #b8b2a9; }
  #outbox-list .where { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #outbox-list li[data-state="delivered"] .state { color: #8ce99a; }
  #outbox-list li[data-state="unsent"] .state { color: #ffa8a8; }
  #outbox-list li[data-state="delivered"] .text,
  #outbox-list li[data-state="unsent"] .text { color: #d8d4cd; }
  #outbox-list .text { margin: 0; overflow-wrap: anywhere; white-space: pre-wrap; }

  #outbox-list .again {
    align-self: flex-start;
    padding: 3px 8px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #2a2a2a;
    color: #fff;
    font: 11px ui-monospace, Menlo, monospace;
  }


  #panel {
    position: fixed;
    right: 14px;
    bottom: 58px;
    z-index: 2147483644;
    width: 232px;
    padding: 6px;
    border: 1px solid rgb(255 255 255 / 12%);
    border-radius: 10px;
    background: #161616;
    color: #fff;
    box-shadow: 0 6px 20px rgb(0 0 0 / 22%);
    font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #panel[hidden] { display: none; }

  .control {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 9px 10px;
    border-radius: 6px;
  }

  .control .label { flex-grow: 1; cursor: pointer; }

  #corners { display: flex; gap: 4px; }

  .corner {
    width: 24px;
    height: 22px;
    padding: 0;
    border: 1px solid #3a3a3a;
    border-radius: 5px;
    background: transparent;
    color: #cfcac2;
    cursor: pointer;
    font: 13px/1 system-ui, sans-serif;
  }

  .corner[aria-pressed="true"] { border-color: #2f9e44; background: #2f9e44; color: #fff; }

  /* The chosen corner (Panel sets data-corner on the host): what sits at
     the bottom right by default moves to that corner. */
  :host([data-corner$="left"]) :is(#bar, #list, #panel, #toast, #agent-general) {
    right: auto;
    left: 14px;
  }

  :host([data-corner^="top"]) :is(#bar, #list) { bottom: auto; top: 14px; }
  :host([data-corner^="top"]) #panel { bottom: auto; top: 58px; }
  :host([data-corner^="top"]) :is(#toast, #agent-general) { bottom: auto; top: 52px; }

  .chip {
    padding: 1px 5px;
    border: 1px solid #3a3a3a;
    border-radius: 4px;
    background: transparent;
    color: #b8b2a9;
    cursor: pointer;
    font: 11px/1.3 ui-monospace, Menlo, monospace;
    white-space: nowrap;
  }

  .chip[data-listening="true"] {
    padding: 1px 6px;
    border: 1px dashed #66d9e8;
    color: #66d9e8;
  }

  .switch {
    display: flex;
    flex: none;
    align-items: center;
    justify-content: flex-start;
    width: 28px;
    height: 16px;
    padding: 2px;
    border: 0;
    border-radius: 999px;
    background: #3a3a3a;
    cursor: pointer;
  }

  .switch::after {
    content: "";
    display: block;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #b8b2a9;
  }

  .switch[aria-checked="true"] { justify-content: flex-end; background: #2f9e44; }
  .switch[aria-checked="true"]::after { background: #fff; }

  #name-row { display: flex; align-items: center; gap: 8px; padding: 6px 10px; }
  #name-row label { color: #b8b2a9; font-weight: 400; margin: 0; }

  #name-field {
    flex-grow: 1;
    min-width: 0;
    padding: 4px 6px;
    border: 1px solid #3a3a3a;
    border-radius: 4px;
    background: #222;
    color: #fff;
    font: 12px/1.3 ui-monospace, Menlo, monospace;
  }

  #name-field:focus { outline: 1px solid #66d9e8; }

  #panel-message {
    padding: 0 10px 6px;
    color: #f0c27a;
    font-size: 11px;
  }

  #panel-message:empty { display: none; }

  #people {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 4px;
    padding: 10px 10px 6px;
    border-top: 1px solid #262626;
  }

  #here-count { color: #8c867e; font-size: 11px; }

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
    width: 24px;
    height: 24px;
    border: 2px solid #161616;
    border-radius: 50%;
    color: #fff;
    font: 700 10px/1 system-ui, -apple-system, sans-serif;
  }

  .avatar + .avatar { margin-left: -6px; }
  .avatar[data-you="true"] { border-color: #fff; background: #161616; }

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
    width: 120px;
    height: 48px;
    opacity: 1;
    transition: opacity 400ms;
    will-change: transform;
  }

  .cursor[data-faded="true"] { opacity: 0; }
  .cursor svg { position: absolute; left: 0; top: 0; filter: drop-shadow(0 1px 2px rgb(0 0 0 / 30%)); }

  .cursor .tag {
    position: absolute;
    left: 13px;
    top: 17px;
    padding: 3px 8px;
    border-radius: 0 6px 6px 6px;
    color: #fff;
    box-shadow: 0 1px 3px rgb(0 0 0 / 25%);
    font: 600 11px/1.3 system-ui, -apple-system, sans-serif;
    white-space: nowrap;
  }

  #name-dialog {
    top: 50%;
    left: 50%;
    width: min(400px, calc(100vw - 24px));
    padding: 22px;
    border-radius: 10px;
    font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    transform: translate(-50%, -50%);
  }

  #name-dialog::backdrop { background: rgb(0 0 0 / 35%); }
  #name-dialog form { display: flex; flex-direction: column; gap: 14px; padding: 0; }
  #name-title { display: flex; flex-direction: column; gap: 4px; }
  #name-title strong { font: 700 20px Georgia, serif; }
  #name-title span, #name-foot span { color: #5b564f; }
  #name-foot { display: flex; align-items: center; justify-content: space-between; gap: 10px; }
  #name-foot span { font-size: 11px; }
  #name-dialog label { margin: 0; }

  #name-input {
    padding: 8px 10px;
    border: 2px solid #287088;
    border-radius: 4px;
    background: #fff;
    color: #161616;
    font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #name-error { color: #a12c10; font-size: 11px; }
  #name-error:empty { display: none; }

  #toast {
    position: fixed;
    right: 14px;
    bottom: 52px;
    z-index: 2147483644;
    max-width: min(360px, calc(100vw - 28px));
    padding: 9px 11px;
    border-radius: 5px;
    background: #161616;
    color: #fff;
    box-shadow: 0 4px 18px rgb(0 0 0 / 24%);
    font: 13px/1.35 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #toast[hidden] { display: none; }

  #agent-log { display: contents; }

  #agent-general {
    position: fixed;
    right: 14px;
    bottom: 52px;
    z-index: 2147483643;
    display: flex;
    width: min(360px, calc(100vw - 28px));
    max-height: calc(100vh - 66px);
    flex-direction: column;
    gap: 8px;
    overflow: auto;
    pointer-events: none;
  }

  #agent-anchored {
    position: fixed;
    inset: 0;
    z-index: 2147483643;
    pointer-events: none;
  }

  .agent-message {
    position: relative;
    width: min(340px, calc(100vw - 16px));
    max-height: calc(100vh - 16px);
    padding: 12px 34px 12px 13px;
    border: 1px solid #45413c;
    border-radius: 7px;
    background: #fffdf8;
    color: #1d1b19;
    box-shadow: 0 6px 24px rgb(0 0 0 / 24%);
    font: 13px/1.45 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    overflow-wrap: anywhere;
    overflow-y: auto;
    pointer-events: auto;
  }

  #agent-anchored .agent-message { position: fixed; }
  .agent-message p { margin: 0 0 8px; }
  .agent-message p:last-child { margin-bottom: 0; }
  .agent-message ul, .agent-message ol { margin: 6px 0; padding-left: 22px; }
  .agent-message pre { overflow: auto; }
  .agent-message code { font: inherit; background: #eeeae2; }
  .agent-message a { color: #075f7a; text-decoration: underline; }
  .agent-message.has-anchor { cursor: pointer; }
  .agent-message.has-anchor:focus-visible {
    outline: 3px solid #287088;
    outline-offset: 2px;
  }

  .agent-dismiss {
    position: absolute;
    top: 5px;
    right: 5px;
    width: 25px;
    height: 25px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: #504b45;
    font: 18px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  .agent-dismiss:hover, .agent-dismiss:focus-visible { background: #eeeae2; }

  .agent-meta {
    margin-bottom: 4px;
    color: #7b3221;
    font-size: 11px;
    font-weight: 700;
  }

  .agent-meta[hidden] { display: none; }

  .agent-unavailable {
    margin-top: 8px;
    color: #7b3221;
    font-size: 11px;
  }

  .agent-unavailable[hidden] { display: none; }

  #message-highlight {
    position: fixed;
    display: none;
    z-index: 2147483646;
    border: 3px solid #0b7896;
    border-radius: 3px;
    background: rgb(11 120 150 / 12%);
    box-shadow: 0 0 0 3px rgb(255 255 255 / 85%);
    pointer-events: none;
  }

  dialog {
    position: fixed;
    inset: auto;
    width: min(420px, calc(100vw - 24px));
    margin: 0;
    padding: 0;
    border: 1px solid #444;
    border-radius: 7px;
    background: #f8f7f4;
    color: #161616;
    box-shadow: 0 16px 48px rgb(0 0 0 / 28%);
  }

  dialog::backdrop { background: rgb(0 0 0 / 6%); }
  form { padding: 14px; }

  label {
    display: block;
    margin-bottom: 8px;
    font-weight: 700;
    line-height: 1.3;
  }

  #selection-preview {
    margin: 0 0 10px;
    padding: 8px 10px;
    border-left: 3px solid #eb5e28;
    background: #eeeae2;
  }

  #selection-preview[hidden] { display: none; }

  #selection-label {
    margin-bottom: 4px;
    color: #655f58;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  #selection-text {
    max-height: 96px;
    margin: 0;
    overflow: auto;
    color: #2b2926;
    font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    white-space: pre-wrap;
  }

  textarea {
    display: block;
    width: 100%;
    min-height: 112px;
    resize: vertical;
    padding: 9px 10px;
    border: 1px solid #888;
    border-radius: 4px;
    background: #fff;
    color: #161616;
    font: 14px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  textarea:focus { outline: 2px solid #287088; outline-offset: 1px; }

  #status {
    min-height: 18px;
    margin-top: 7px;
    color: #a12c10;
    font-size: 11px;
    line-height: 1.4;
  }

  #reload-notice {
    margin-top: 7px;
    color: #655f58;
    font-size: 11px;
    line-height: 1.4;
  }

  #reload-notice[hidden] { display: none; }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 10px;
  }

  button {
    padding: 7px 11px;
    border: 1px solid #555;
    border-radius: 4px;
    background: #fff;
    color: #161616;
    cursor: pointer;
    font: 600 12px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  button[type="submit"] { background: #161616; color: #fff; }
  button:disabled { cursor: wait; opacity: 0.55; }
`;

// client/src/ui.ts
var GEAR_ICON = `<svg aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"></circle><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"></path></svg>`;
var SELECT_ICON = `<svg aria-hidden="true" width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 3l7.5 18 2.5-7.5L21.5 11z"></path></svg>`;
var MARKUP = `
  <style>${styles}</style>
  <div id="cursors" aria-hidden="true"></div>
  <div id="highlight" aria-hidden="true"></div>
  <div id="edit-highlight" aria-hidden="true"></div>
  <div id="message-highlight" aria-hidden="true"></div>
  <div id="bar">
    <button id="pill" type="button" aria-expanded="false" aria-controls="list">
      <span id="connection" class="dot" data-state="amber" role="img" aria-label="Connecting" title="Connecting"></span>
      <span id="badge" role="status" aria-live="polite">0 queued</span>
    </button>
    <button id="edit" type="button" aria-label="Edit mode" aria-pressed="false" title="Edit mode: click an element to comment on it (Esc leaves)">${SELECT_ICON}</button>
    <button id="gear" type="button" aria-label="Settings" aria-expanded="false" aria-controls="panel">${GEAR_ICON}</button>
  </div>
  <section id="list" aria-label="Your comments" hidden>
    <header id="list-header">
      <div id="list-summary">
        <span id="list-dot" class="dot" data-state="amber"></span>
        <span id="list-count">0 queued</span>
        <span id="connection-text" role="status">· connecting</span>
      </div>
      <button id="collapse" type="button" aria-label="Collapse">▾</button>
    </header>
    <ol id="outbox-list"></ol>
  </section>
  <section id="panel" aria-label="jampot settings" hidden>
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
    <div class="control" id="corner-row">
      <span class="label" id="corner-label">Corner</span>
      <span id="corners" role="group" aria-labelledby="corner-label">
        <button class="corner" type="button" data-corner="top-left" aria-label="Top left" aria-pressed="false">↖</button>
        <button class="corner" type="button" data-corner="top-right" aria-label="Top right" aria-pressed="false">↗</button>
        <button class="corner" type="button" data-corner="bottom-left" aria-label="Bottom left" aria-pressed="false">↙</button>
        <button class="corner" type="button" data-corner="bottom-right" aria-label="Bottom right" aria-pressed="true">↘</button>
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
  <dialog id="jampot-dialog" aria-labelledby="dialog-label">
    <form>
      <label id="dialog-label" for="comment">Comment:</label>
      <div id="selection-preview" hidden>
        <div id="selection-label">Selected text</div>
        <blockquote id="selection-text"></blockquote>
      </div>
      <textarea id="comment" name="comment" maxlength="${MAX_COMMENT}" required></textarea>
      <div id="status" role="alert"></div>
      <div id="reload-notice" role="status" hidden>
        Files changed. Reloading when this edit closes.
      </div>
      <div class="actions">
        <button id="cancel" type="button">Cancel</button>
        <button id="send" type="submit">Queue comment</button>
      </div>
    </form>
  </dialog>
  <dialog id="name-dialog" aria-labelledby="name-heading">
    <form id="name-form">
      <div id="name-title">
        <strong id="name-heading">Join the potluck.</strong>
        <span>Your name is shown next to your comments and your cursor.</span>
      </div>
      <label for="name-input">Name</label>
      <input id="name-input" type="text" maxlength="${MAX_NAME}" autocomplete="nickname" spellcheck="false" required>
      <div id="name-error" role="alert"></div>
      <div id="name-foot">
        <span>Remembered on this computer; change it under the gear.</span>
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
  panel;
  commentModeKey;
  commentMode;
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
  dialogLabel;
  form;
  selectionPreview;
  selectionText;
  textarea;
  status;
  reloadNotice;
  cancel;
  send;
  nameDialog;
  nameForm;
  nameInput;
  nameError;
  nameJoin;
  toastTimer;
  /** Adds the UI to the page; null when it is there already. */
  static install() {
    if (document.getElementById(HOST_ID)) return null;
    const host = document.createElement("div");
    host.id = HOST_ID;
    const shadow = host.attachShadow({ mode: "open" });
    shadow.innerHTML = MARKUP;
    document.body.append(host);
    return new _Ui(host, shadow);
  }
  constructor(host, shadow) {
    const find = (selector, type) => {
      const element = shadow.querySelector(selector);
      if (!(element instanceof type)) throw new Error(`jampot: missing ${selector}`);
      return element;
    };
    this.host = host;
    this.shadow = shadow;
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
    this.panel = find("#panel", HTMLElement);
    this.commentModeKey = find("#comment-mode-key", HTMLButtonElement);
    this.commentMode = find("#comment-mode", HTMLButtonElement);
    this.showCursorKey = find("#show-cursor-key", HTMLButtonElement);
    this.showCursor = find("#show-cursor", HTMLButtonElement);
    this.nameField = find("#name-field", HTMLInputElement);
    this.panelMessage = find("#panel-message", HTMLDivElement);
    this.hereCount = find("#here-count", HTMLSpanElement);
    this.avatars = find("#avatars", HTMLUListElement);
    this.toast = find("#toast", HTMLDivElement);
    this.agentGeneral = find("#agent-general", HTMLDivElement);
    this.agentAnchored = find("#agent-anchored", HTMLDivElement);
    this.dialog = find("#jampot-dialog", HTMLDialogElement);
    this.dialogLabel = find("#dialog-label", HTMLLabelElement);
    this.form = find("#jampot-dialog form", HTMLFormElement);
    this.selectionPreview = find("#selection-preview", HTMLDivElement);
    this.selectionText = find("#selection-text", HTMLQuoteElement);
    this.textarea = find("textarea", HTMLTextAreaElement);
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
  setPending(pending) {
    const text = `${pending} queued`;
    this.badge.textContent = text;
    this.badge.dataset.pending = String(pending);
    this.listCount.textContent = text;
  }
};

// client/src/main.ts
function install() {
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
  const outbox = new Outbox((submission) => socket.request({ type: "comment", ...submission }));
  const changeSettings = (change) => socket.request({ type: "settings", change });
  const answers = new Answers(ui);
  const dialog = new CommentDialog(ui, outbox, () => pending);
  const list = new CommentList(ui, outbox);
  const panel = new Panel(ui, changeSettings);
  const sender = new CursorSender(
    ui,
    (position) => socket.send({ type: "cursor", position })
  );
  const layer = new CursorLayer(ui);
  const editMode = new EditMode(
    ui,
    dialog,
    () => panel.commentMode
  );
  editMode.onPagePress = () => {
    panel.close();
    list.close();
  };
  list.onOpen = () => {
    panel.close();
  };
  panel.onOpen = () => {
    list.close();
  };
  outbox.onChange = () => {
    list.render();
  };
  const prompt = new NamePrompt(ui, changeSettings);
  panel.onSettings = (settings) => {
    sender.setOn(settings.showCursor === "on");
    prompt.show(settings);
    editMode.refresh();
  };
  const connection = new Connection((view, reached) => {
    list.showState(view);
    if (view.colour !== "green") layer.clear();
    if (reached) void outbox.resendAll();
  });
  const showPeople = (people) => {
    here = people;
    panel.showPeople(here, you);
    layer.setPeople(here, url);
  };
  const showMine = (undelivered) => {
    if (undelivered === null) return;
    pending = undelivered.length;
    ui.setPending(pending);
    outbox.sync(undelivered);
  };
  const onRevision = (revision) => {
    if (shownRevision === null || revision === null || revision <= shownRevision) return;
    shownRevision = revision;
    dialog.reloadWhenClosed();
  };
  let restored = false;
  const restoreDraft = () => {
    if (restored) return;
    restored = true;
    dialog.restore((selector) => answers.targetFor(selector));
  };
  function applyHello(hello) {
    you = hello.link.you?.key ?? null;
    showMine(hello.mine);
    for (const answer of hello.answers) answers.add(answer);
    restoreDraft();
    panel.show(hello.settings);
    showPeople(hello.people);
    onRevision(hello.revision);
    connection.setLink(hello.link);
  }
  function applySlice(slice) {
    switch (slice.type) {
      case "link":
        you = slice.you?.key ?? null;
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
      case "answer":
        answers.add(slice);
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
    event.preventDefault();
    dialog.openElement(event.target, event.clientX, event.clientY);
  }, true);
  ui.shadow.addEventListener("contextmenu", (event) => {
    if (!commenting()) return;
    if (!(event instanceof MouseEvent) || event.shiftKey) return;
    const target = event.composedPath().find(
      (node) => node instanceof HTMLElement && node.classList.contains("agent-message")
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
    dialog.openPage();
  }, true);
  list.render();
  if (loadDraft()?.target.kind !== "answer") restoreDraft();
  socket.start();
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", install, { once: true });
} else {
  install();
}
