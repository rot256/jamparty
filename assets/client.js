// Generated from client/src by `npm run build`; do not edit.

// client/src/config.ts
var SCRIPT_URL = new URL(import.meta.url);
var BASE = new URL("./", SCRIPT_URL);
var COMMENTS_URL = new URL("api/comments", BASE);
var STATUS_URL = new URL("api/status", BASE);
var EVENTS_URL = new URL("api/events", BASE);
var LIVE_REVISION = SCRIPT_URL.searchParams.get("live");
var SHARE_VERSION = SCRIPT_URL.searchParams.get("version");
var HOST_ID = "jampot";
var MAX_SELECTED_TEXT = 2e3;
var DISMISSED_KEY = "jampot-dismissed-v1";
var OUTBOX_KEY = "jampot-outbox-v1";
var MAX_OUTBOX = 20;
var WATCHDOG_MS = 35e3;
var AGENT_TARGET_PREFIX = "jampot-agent:";

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
    const body = document.createElement("div");
    body.className = "agent-body";
    body.innerHTML = answer.html;
    const unavailable = document.createElement("div");
    unavailable.className = "agent-unavailable";
    unavailable.textContent = "Target unavailable";
    unavailable.hidden = true;
    node.append(close, body, unavailable);
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

// client/src/generated/api.ts
var API_VERSION = 1;

// client/src/api.ts
var SubmitError = class extends Error {
  constructor(message, unsent) {
    super(message);
    this.unsent = unsent;
  }
  unsent;
};
function isObject(value) {
  return typeof value === "object" && value !== null;
}
function errorMessage(body, fallback) {
  return isObject(body) && typeof body.error === "string" ? body.error : fallback;
}
async function submit(submission) {
  let response;
  try {
    response = await fetch(COMMENTS_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(submission)
    });
  } catch {
    throw new SubmitError("jampot join is not reachable", true);
  }
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    throw new SubmitError(
      errorMessage(body, `request failed (${response.status})`),
      response.status === 503
    );
  }
  return body;
}
async function fetchStatus() {
  try {
    const response = await fetch(STATUS_URL);
    if (!response.ok) return null;
    return checkVersion(await response.json());
  } catch {
    return null;
  }
}
function checkVersion(body) {
  const api = isObject(body) ? body.api : void 0;
  if (api === API_VERSION) return { status: body };
  const share = SHARE_VERSION ?? "unknown";
  const joiner = isObject(body) && typeof body.version === "string" ? body.version : "another";
  const incompatible = typeof api === "number" ? `jampot versions do not match: the share runs ${share} (page API ${API_VERSION}), your jampot join runs ${joiner} (page API ${api}). Use the same jampot version on both sides and join again.` : `Your jampot join is older than the share's jampot ${share} and cannot serve this page. Use the same jampot version on both sides and join again.`;
  return { incompatible };
}
function openEvents() {
  return new EventSource(EVENTS_URL);
}
function parseEvent(data) {
  try {
    const event = JSON.parse(data);
    return isObject(event) && typeof event.type === "string" ? event : null;
  } catch {
    return null;
  }
}

// client/src/connection.ts
var Connection = class {
  constructor(handlers) {
    this.handlers = handlers;
  }
  handlers;
  events = null;
  lastEvent = Date.now();
  streamOpen = false;
  shareConnected = false;
  statusLine = null;
  incompatible = null;
  colour = "amber";
  watchdog;
  /** Checks the joiner's page API, then connects the event stream. */
  async start() {
    const checked = await fetchStatus();
    if (checked !== null && "incompatible" in checked) {
      this.refuse(checked.incompatible);
      return;
    }
    this.connect();
    this.watchdog = window.setInterval(() => {
      if (Date.now() - this.lastEvent < WATCHDOG_MS) return;
      this.events?.close();
      this.streamOpen = false;
      this.render();
      this.lastEvent = Date.now();
      this.connect();
    }, 5e3);
  }
  get view() {
    let colour = "green";
    let text = "connected";
    if (this.incompatible !== null) {
      colour = "grey";
      text = this.incompatible;
    } else if (!this.streamOpen) {
      colour = "amber";
      text = "Reconnecting to jampot join…";
    } else if (!this.shareConnected) {
      colour = "grey";
      text = "The share is unreachable";
    } else if (this.statusLine !== null) {
      colour = "grey";
      text = this.statusLine;
    }
    return { colour, text, incompatible: this.incompatible };
  }
  /** Whether comments can go out now. */
  get connected() {
    return this.colour === "green";
  }
  refuse(reason) {
    this.incompatible = reason;
    this.events?.close();
    this.events = null;
    if (this.watchdog !== void 0) clearInterval(this.watchdog);
    this.render();
  }
  render() {
    const view = this.view;
    const recovered = view.colour === "green" && this.colour !== "green";
    this.colour = view.colour;
    this.handlers.state(view, recovered);
  }
  /** Reads `api/status` again (on every reconnection and after deliveries). */
  async refresh() {
    const checked = await fetchStatus();
    if (checked === null) return;
    if ("incompatible" in checked) {
      this.refuse(checked.incompatible);
      return;
    }
    const status = checked.status;
    this.shareConnected = status.connected;
    this.statusLine = status.status;
    this.handlers.status(status);
    this.render();
  }
  connect() {
    if (this.incompatible !== null) return;
    const events = openEvents();
    this.events = events;
    events.addEventListener("open", () => {
      this.streamOpen = true;
      this.lastEvent = Date.now();
      this.render();
      void this.refresh();
    });
    events.addEventListener("error", () => {
      this.streamOpen = false;
      this.render();
    });
    events.addEventListener("message", (message) => {
      this.lastEvent = Date.now();
      const event = parseEvent(message.data);
      if (event === null) return;
      switch (event.type) {
        case "share":
          this.shareConnected = event.connected;
          this.render();
          if (event.connected) void this.refresh();
          break;
        case "status":
          this.statusLine = event.line;
          this.render();
          break;
        case "delivered":
          void this.refresh();
          break;
        case "answer":
        case "reload":
        case "presence":
        case "ping":
          break;
      }
      this.handlers.event(event);
    });
  }
};

// client/src/dialog.ts
var ELEMENT_LABEL = "Comment:";
var ANSWER_LABEL = "Comment on agent response:";
var CommentDialog = class {
  constructor(ui, outbox) {
    this.ui = ui;
    this.outbox = outbox;
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
    cancel.addEventListener("click", () => {
      this.close();
    });
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      this.close();
    });
    dialog.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      const rect = dialog.getBoundingClientRect();
      if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) this.close();
    });
    dialog.addEventListener("close", () => {
      this.stopTracking();
      ui.highlight.style.display = "none";
    });
  }
  ui;
  outbox;
  captured = null;
  /** A live reload waiting for the dialog to close. */
  pendingReload = false;
  get open() {
    return this.ui.dialog.open;
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
  openElement(target, x, y, selector, label = ELEMENT_LABEL) {
    this.show(captureTarget(target, selector), x, y, label);
  }
  openPage() {
    this.show(capturePageTarget(), window.innerWidth / 2, window.innerHeight / 3, ELEMENT_LABEL);
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
  show(captured, x, y, label) {
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
  close() {
    const ui = this.ui;
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
  submit() {
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
      comment
    };
    const ref = crypto.randomUUID();
    const at = Date.now();
    submit({ ref, ...draft }).then((queued) => {
      this.outbox.add({ draft, ref, id: queued.id, state: "queued", at });
      this.outbox.onPending(queued.pending);
      this.close();
      ui.showToast(`Comment queued · ${queued.pending} pending`);
    }).catch((error) => {
      if (error instanceof SubmitError && error.unsent) {
        this.outbox.add({ draft, ref, id: null, state: "unsent", at });
        this.close();
        ui.showToast("Not sent: kept in the page, sent when the connection is back");
        return;
      }
      ui.status.textContent = error instanceof Error ? error.message : "Could not queue comment";
      ui.textarea.focus();
    }).finally(() => {
      ui.send.disabled = false;
      ui.cancel.disabled = false;
    });
  }
};

// client/src/outbox.ts
function isRecord(value) {
  return typeof value === "object" && value !== null;
}
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
  entries = load();
  /** Called after every change, to redraw. */
  onChange = () => void 0;
  /** Called with the joiner's pending count after a comment was queued. */
  onPending = () => void 0;
  add(entry) {
    this.entries.push(entry);
    this.changed();
  }
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
  async resend(entry) {
    if (entry.state !== "unsent") return;
    entry.state = "sending";
    this.changed();
    try {
      const queued = await submit({ ref: entry.ref, ...entry.draft });
      entry.id = queued.id;
      entry.state = "queued";
      this.onPending(queued.pending);
    } catch {
      entry.state = "unsent";
    }
    this.changed();
  }
  async resendAll() {
    for (const entry of this.entries.filter((candidate) => candidate.state === "unsent")) {
      await this.resend(entry);
    }
  }
  markDelivered(ids) {
    const delivered = new Set(ids);
    for (const entry of this.entries) {
      if (entry.state === "queued" && entry.id !== null && delivered.has(entry.id)) {
        entry.state = "delivered";
      }
    }
    this.changed();
  }
  /** Every queued comment not in `undelivered` was delivered meanwhile. */
  markDeliveredExcept(undelivered) {
    const held = new Set(undelivered);
    this.markDelivered(
      this.entries.filter((entry) => entry.state === "queued" && entry.id !== null && !held.has(entry.id)).map((entry) => entry.id ?? "")
    );
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

  #bar {
    position: fixed;
    right: 14px;
    bottom: 14px;
    z-index: 2147483644;
    display: flex;
    align-items: center;
    gap: 6px;
    pointer-events: none;
  }

  #connection {
    width: 12px;
    height: 12px;
    border: 2px solid #fff;
    border-radius: 50%;
    background: #9a958e;
    box-shadow: 0 1px 6px rgb(0 0 0 / 30%);
    pointer-events: auto;
  }

  #connection[data-state="green"] { background: #2f9e44; }
  #connection[data-state="amber"] { background: #e8a33d; }
  #connection[data-state="grey"] { background: #9a958e; }

  #outbox {
    position: fixed;
    left: 14px;
    bottom: 14px;
    z-index: 2147483644;
    width: min(320px, calc(100vw - 28px));
    max-height: 40vh;
    overflow: auto;
    padding: 8px 10px;
    border-radius: 6px;
    background: #161616;
    color: #fff;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    font: 12px/1.4 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  }

  #outbox[hidden] { display: none; }
  #connection-text { color: #f0c27a; }
  #connection-text[hidden] { display: none; }
  #outbox ul { margin: 0; padding: 0; list-style: none; }
  #outbox li { display: flex; gap: 8px; align-items: baseline; margin-top: 4px; }
  #outbox .text { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  #outbox .state { color: #b8b2a9; }
  #outbox li[data-state="delivered"] .state { color: #8ce99a; }
  #outbox li[data-state="unsent"] .state { color: #ffa8a8; }
  #outbox button { padding: 3px 6px; font-size: 11px; }

  #badge {
    padding: 7px 10px;
    border: 1px solid rgb(255 255 255 / 30%);
    border-radius: 999px;
    background: #161616;
    color: #fff;
    box-shadow: 0 3px 12px rgb(0 0 0 / 22%);
    font: 600 12px/1 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
    pointer-events: none;
  }

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
var MARKUP = `
  <style>${styles}</style>
  <div id="highlight" aria-hidden="true"></div>
  <div id="message-highlight" aria-hidden="true"></div>
  <div id="bar">
    <div id="connection" data-state="amber" role="img" aria-label="Connecting" title="Connecting"></div>
    <div id="badge" role="status" aria-live="polite">0 queued</div>
  </div>
  <div id="outbox" hidden>
    <div id="connection-text" role="status" hidden></div>
    <ul id="outbox-list"></ul>
  </div>
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
      <textarea id="comment" name="comment" maxlength="10000" required></textarea>
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
`;
var Ui = class _Ui {
  host;
  shadow;
  highlight;
  messageHighlight;
  badge;
  connection;
  outbox;
  connectionText;
  outboxList;
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
    this.highlight = find("#highlight", HTMLDivElement);
    this.messageHighlight = find("#message-highlight", HTMLDivElement);
    this.badge = find("#badge", HTMLDivElement);
    this.connection = find("#connection", HTMLDivElement);
    this.outbox = find("#outbox", HTMLDivElement);
    this.connectionText = find("#connection-text", HTMLDivElement);
    this.outboxList = find("#outbox-list", HTMLUListElement);
    this.toast = find("#toast", HTMLDivElement);
    this.agentGeneral = find("#agent-general", HTMLDivElement);
    this.agentAnchored = find("#agent-anchored", HTMLDivElement);
    this.dialog = find("dialog", HTMLDialogElement);
    this.dialogLabel = find("#dialog-label", HTMLLabelElement);
    this.form = find("form", HTMLFormElement);
    this.selectionPreview = find("#selection-preview", HTMLDivElement);
    this.selectionText = find("#selection-text", HTMLQuoteElement);
    this.textarea = find("textarea", HTMLTextAreaElement);
    this.status = find("#status", HTMLDivElement);
    this.reloadNotice = find("#reload-notice", HTMLDivElement);
    this.cancel = find("#cancel", HTMLButtonElement);
    this.send = find("#send", HTMLButtonElement);
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
    this.badge.textContent = `${pending} queued`;
    this.badge.dataset.pending = String(pending);
  }
};

// client/src/main.ts
function stateText(entry) {
  return entry.state === "unsent" ? "not sent" : entry.state;
}
function isEditing(event) {
  return event.composedPath().some(
    (node) => node instanceof HTMLElement && (node.isContentEditable || node.matches("input, textarea, select"))
  );
}
function install() {
  const ui = Ui.install();
  if (ui === null) return;
  const outbox = new Outbox();
  const answers = new Answers(ui);
  const dialog = new CommentDialog(ui, outbox);
  let reloadRevision = LIVE_REVISION;
  let connectionView = { colour: "amber", text: "", incompatible: null };
  const renderOutbox = () => {
    ui.outboxList.replaceChildren(...outbox.entries.map((entry) => {
      const item = document.createElement("li");
      item.dataset.state = entry.state;
      const text = document.createElement("span");
      text.className = "text";
      text.textContent = entry.draft.comment;
      const state = document.createElement("span");
      state.className = "state";
      state.textContent = stateText(entry);
      item.append(text, state);
      if (entry.state === "unsent") {
        const again = document.createElement("button");
        again.type = "button";
        again.textContent = "send again";
        again.addEventListener("click", () => {
          void outbox.resend(entry);
        });
        item.append(again);
      }
      return item;
    }));
    ui.outbox.hidden = outbox.entries.length === 0 && ui.connectionText.hidden;
  };
  outbox.onChange = renderOutbox;
  outbox.onPending = (pending) => {
    ui.setPending(pending);
  };
  const onRevision = (revision) => {
    if (LIVE_REVISION === null || revision === null) return;
    if (String(revision) === reloadRevision) return;
    reloadRevision = String(revision);
    dialog.reloadWhenClosed();
  };
  const connection = new Connection({
    state(view, recovered) {
      connectionView = view;
      ui.connection.dataset.state = view.colour;
      ui.connection.title = view.text;
      ui.connection.setAttribute("aria-label", view.text);
      ui.connectionText.textContent = view.text;
      ui.connectionText.hidden = view.colour === "green";
      renderOutbox();
      if (recovered) void outbox.resendAll();
    },
    status(status) {
      ui.setPending(status.pending);
      outbox.markDeliveredExcept(status.undelivered);
      for (const answer of status.answers) answers.add(answer);
      onRevision(status.revision);
    },
    event(event) {
      switch (event.type) {
        case "answer":
          answers.add(event);
          break;
        case "reload":
          onRevision(event.revision);
          break;
        case "delivered":
          outbox.markDelivered(event.ids);
          break;
        case "share":
        case "status":
        case "presence":
        case "ping":
          break;
      }
    }
  });
  document.addEventListener("contextmenu", (event) => {
    if (connectionView.incompatible !== null) return;
    if (event.shiftKey || event.composedPath().includes(ui.host)) return;
    if (!(event.target instanceof Element)) return;
    event.preventDefault();
    dialog.openElement(event.target, event.clientX, event.clientY);
  }, true);
  ui.shadow.addEventListener("contextmenu", (event) => {
    if (connectionView.incompatible !== null) return;
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
      `${AGENT_TARGET_PREFIX}${target.dataset.messageId}`,
      ANSWER_LABEL
    );
  }, true);
  document.addEventListener("keydown", (event) => {
    if (connectionView.incompatible !== null) return;
    if (event.defaultPrevented || event.isComposing || event.key !== "/" || event.metaKey || event.ctrlKey || event.altKey) return;
    if (isEditing(event)) return;
    event.preventDefault();
    dialog.openPage();
  }, true);
  renderOutbox();
  void connection.start();
}
if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", install, { once: true });
} else {
  install();
}
