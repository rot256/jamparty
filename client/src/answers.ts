// The agent's answers: shown bottom right, or next to the element of the
// comment they answer when that is on this page; dismissed ones stay
// dismissed for the session. At most MAX_ANSWERS are shown, as many as
// the joiner keeps, the oldest going first; dismissals are kept only of
// answers the joiner still has, and at most as many.

import { samePage } from "./capture";
import { AGENT_TARGET_PREFIX, DISMISSED_KEY } from "./config";
import { MAX_ANSWERS } from "./generated/api";
import type { Answer, CommentOutcome, Submission } from "./generated/api";
import type { Ui } from "./ui";

interface Shown {
  node: HTMLElement;
  unavailable: HTMLElement;
  /** The selector it belongs next to; null when it is not anchored here. */
  anchor: string | null;
  stopTimer: () => void;
}

interface Box {
  left: number;
  right: number;
  top: number;
  bottom: number;
}

const GAP = 8;
const EDGE = 8;

function loadDismissed(): Set<string> {
  try {
    const stored: unknown = JSON.parse(sessionStorage.getItem(DISMISSED_KEY) ?? "[]");
    const ids = Array.isArray(stored)
      ? stored.filter((id): id is string => typeof id === "string")
      : [];
    return new Set(ids.slice(-MAX_ANSWERS));
  } catch {
    return new Set();
  }
}

export class Answers {
  private readonly shown = new Map<string, Shown>();
  private readonly dismissed = loadDismissed();
  private placementFrame: number | undefined;
  private timeoutSeconds = 0;
  private highlightTimer: number | undefined;

  constructor(
    private readonly ui: Ui,
    private readonly pick: (submission: Omit<Submission, "n">) => Promise<CommentOutcome>,
  ) {
    const schedule = (): void => { this.schedulePlacement(); };
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    new MutationObserver(schedule).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      characterData: true,
    });
  }

  /** The node of answer `id`, for comments on an answer. */
  node(id: string): HTMLElement | null {
    return this.shown.get(id)?.node ?? null;
  }

  /** The element a selector names: on the page, or one of the answers. */
  targetFor(selector: string): Element | null {
    if (selector.startsWith(AGENT_TARGET_PREFIX)) {
      return this.node(selector.slice(AGENT_TARGET_PREFIX.length));
    }
    return document.querySelector(selector);
  }

  /**
   * The answers the joiner has now (its hello): dismissals of others are
   * forgotten, since those answers do not come back.
   */
  keepDismissed(answers: readonly Answer[]): void {
    const held = new Set(answers.map((answer) => answer.id));
    for (const id of this.shown.keys()) if (!held.has(id)) this.remove(id);
    let changed = false;
    for (const id of [...this.dismissed]) {
      if (!held.has(id)) {
        this.dismissed.delete(id);
        changed = true;
      }
    }
    if (changed) this.persistDismissed();
  }

  add(answer: Answer): void {
    if (this.dismissed.has(answer.id) || this.shown.has(answer.id)) return;
    // The oldest go first (a Map keeps the order answers came in).
    for (const [id, shown] of this.shown) {
      if (this.shown.size < MAX_ANSWERS) break;
      shown.stopTimer();
      shown.node.remove();
      this.shown.delete(id);
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
    const body = document.createElement("div");
    body.className = "agent-body";
    // Rendered by the joiner with the restricted Markdown renderer.
    body.innerHTML = answer.html;
    const unavailable = document.createElement("div");
    unavailable.className = "agent-unavailable";
    unavailable.textContent = "Target unavailable";
    unavailable.hidden = true;
    node.append(close, to, body, unavailable);
    const options = document.createElement("div");
    options.className = "agent-choices";
    // Labels are data, never markup. The joiner already checked their caps.
    for (const choice of answer.choices) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = choice;
      const ref = crypto.randomUUID();
      button.addEventListener("click", () => {
        for (const option of options.querySelectorAll("button")) option.disabled = true;
        void this.pick({
          ref, page: location.href, selector: `${AGENT_TARGET_PREFIX}${answer.id}`,
          selectedText: null, comment: choice, choice,
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

    const anchor = answer.anchor !== undefined && samePage(answer.page) ? answer.anchor : null;
    if (anchor !== null) {
      node.classList.add("has-anchor");
      node.tabIndex = 0;
      node.setAttribute("aria-label", "Agent message; activate to show its anchor");
      node.addEventListener("click", (event) => {
        const interactive = event.target instanceof Element
          ? event.target.closest("a, button")
          : null;
        const selection = window.getSelection();
        if (interactive || (selection && !selection.isCollapsed)) return;
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
      if (!href.toLowerCase().startsWith("jamparty:")) {
        link.target = "_blank";
        link.rel = "noopener noreferrer";
      }
    }
    body.addEventListener("click", (event) => {
      const link = event.target instanceof Element ? event.target.closest("a") : null;
      if (link) this.followElementLink(link, event);
    });
    close.addEventListener("click", () => {
      this.shown.get(answer.id)?.stopTimer();
      this.dismissed.add(answer.id);
      for (const id of this.dismissed) {
        if (this.dismissed.size <= MAX_ANSWERS) break;
        this.dismissed.delete(id);
      }
      this.persistDismissed();
      this.shown.delete(answer.id);
      node.remove();
      this.schedulePlacement();
    });
    const shown: Shown = { node, unavailable, anchor, stopTimer: () => undefined };
    this.shown.set(answer.id, shown);
    this.startTimer(shown);
    this.schedulePlacement();
  }

  /** The share resolved this question; no session dismissal is needed. */
  remove(id: string): void {
    const shown = this.shown.get(id);
    shown?.stopTimer();
    shown?.node.remove();
    this.shown.delete(id);
    this.schedulePlacement();
  }

  /** Uses the same dismissal path as the close buttons. */
  clear(): void {
    for (const { node } of this.shown.values()) {
      node.querySelector<HTMLButtonElement>(".agent-dismiss")?.click();
    }
  }

  /** A changed duration gives every visible answer a fresh countdown. */
  setTimeoutSeconds(seconds: number): void {
    if (seconds === this.timeoutSeconds) return;
    this.timeoutSeconds = seconds;
    for (const shown of this.shown.values()) this.startTimer(shown);
  }

  private startTimer(shown: Shown): void {
    shown.stopTimer();
    if (this.timeoutSeconds === 0) return;
    const { node } = shown;
    const bar = document.createElement("div");
    bar.className = "agent-timeout";
    bar.setAttribute("aria-hidden", "true");
    bar.style.animationDuration = `${this.timeoutSeconds}s`;
    bar.style.setProperty("--answer-timeout-steps", String(this.timeoutSeconds));
    node.append(bar);
    let stopped = false;
    let remaining = this.timeoutSeconds * 1000;
    let started: number | undefined;
    let timer: number | undefined;
    let hovering = node.matches(":hover");
    const update = (): void => {
      if (stopped) return;
      const paused = hovering || node.contains(this.ui.shadow.activeElement);
      if (paused && started !== undefined) {
        remaining = Math.max(0, remaining - (performance.now() - started));
        started = undefined;
        clearTimeout(timer);
        timer = undefined;
      } else if (!paused && started === undefined) {
        started = performance.now();
        timer = window.setTimeout(() => {
          node.querySelector<HTMLButtonElement>(".agent-dismiss")?.click();
        }, remaining);
      }
      bar.style.animationPlayState = paused ? "paused" : "running";
    };
    const enter = (): void => { hovering = true; update(); };
    const leave = (): void => { hovering = false; update(); };
    // focusout fires before the new activeElement is installed.
    const focus = (): void => { queueMicrotask(update); };
    node.addEventListener("mouseenter", enter);
    node.addEventListener("mouseleave", leave);
    node.addEventListener("focusin", focus);
    node.addEventListener("focusout", focus);
    shown.stopTimer = () => {
      stopped = true;
      clearTimeout(timer);
      node.removeEventListener("mouseenter", enter);
      node.removeEventListener("mouseleave", leave);
      node.removeEventListener("focusin", focus);
      node.removeEventListener("focusout", focus);
      bar.remove();
      shown.stopTimer = () => undefined;
    };
    update();
  }

  private persistDismissed(): void {
    try {
      sessionStorage.setItem(DISMISSED_KEY, JSON.stringify(Array.from(this.dismissed)));
    } catch {
      // Dismissal remains active for this document if storage is unavailable.
    }
  }

  /** The element an anchored answer belongs next to; undefined when it is not shown. */
  private anchorTarget(anchor: string): Element | undefined {
    try {
      const target = this.targetFor(anchor);
      if (!target || target === this.ui.host || target.getClientRects().length === 0) {
        return undefined;
      }
      return target;
    } catch {
      return undefined;
    }
  }

  schedulePlacement(): void {
    if (this.placementFrame !== undefined) return;
    this.placementFrame = requestAnimationFrame(() => { this.place(); });
  }

  private place(): void {
    this.placementFrame = undefined;
    const { agentAnchored, agentGeneral } = this.ui;
    const groups = new Map<Element, HTMLElement[]>();
    for (const entry of this.shown.values()) {
      const target = entry.anchor === null ? undefined : this.anchorTarget(entry.anchor);
      if (target !== undefined) {
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

    const occupied: Box[] = Array.from(agentGeneral.children).map((node) => {
      const rect = node.getBoundingClientRect();
      return { left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom };
    });
    const overlaps = (candidate: Box): boolean => occupied.some((rect) => !(
      candidate.right + GAP <= rect.left ||
      candidate.left >= rect.right + GAP ||
      candidate.bottom + GAP <= rect.top ||
      candidate.top >= rect.bottom + GAP
    ));
    const placement = (rect: DOMRect, width: number, height: number, desiredTop: number): Box => {
      const maximumLeft = window.innerWidth - width - EDGE;
      const right = Math.max(EDGE, Math.min(rect.right + GAP, maximumLeft));
      const left = Math.max(EDGE, Math.min(rect.left - width - GAP, maximumLeft));
      const horizontal = rect.right + GAP + width <= window.innerWidth - EDGE
        ? [right, left]
        : [left, right];
      const maximumTop = window.innerHeight - height - EDGE;
      const vertical = [
        desiredTop,
        ...occupied.flatMap((placed) => [placed.bottom + GAP, placed.top - height - GAP]),
      ].map((top) => Math.max(EDGE, Math.min(top, maximumTop)));
      const candidates = horizontal
        .flatMap((candidateLeft, side) => vertical.map((candidateTop) => ({
          left: candidateLeft,
          right: candidateLeft + width,
          top: candidateTop,
          bottom: candidateTop + height,
          score: Math.abs(candidateTop - desiredTop) * 2 + side,
        })))
        .sort((first, second) => first.score - second.score);
      const fallback = { left: EDGE, right: EDGE + width, top: desiredTop, bottom: desiredTop + height };
      return candidates.find((candidate) => !overlaps(candidate)) ?? candidates[0] ?? fallback;
    };
    for (const [target, nodes] of groups) {
      const rect = target.getBoundingClientRect();
      const heights = nodes.map((node) => node.offsetHeight);
      const totalHeight = heights.reduce((total, height) => total + height, 0) +
        GAP * Math.max(0, nodes.length - 1);
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

  private showHighlight(target: Element): void {
    const { messageHighlight } = this.ui;
    const rect = target.getBoundingClientRect();
    messageHighlight.style.display = "block";
    messageHighlight.style.left = `${rect.left}px`;
    messageHighlight.style.top = `${rect.top}px`;
    messageHighlight.style.width = `${rect.width}px`;
    messageHighlight.style.height = `${rect.height}px`;
    if (this.highlightTimer !== undefined) clearTimeout(this.highlightTimer);
    this.highlightTimer = window.setTimeout(() => {
      messageHighlight.style.display = "none";
    }, 1800);
  }

  /** Scrolls to the element `selector` names and highlights it briefly. */
  reveal(selector: string): void {
    let target: Element | null;
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
    requestAnimationFrame(() => { this.showHighlight(found); });
  }

  private followElementLink(link: HTMLAnchorElement, event: Event): void {
    const href = link.getAttribute("href") ?? "";
    if (!href.toLowerCase().startsWith("jamparty:")) return;
    event.preventDefault();
    let selector: string;
    try {
      selector = decodeURIComponent(href.slice("jamparty:".length));
    } catch {
      this.ui.showToast("Element link is invalid");
      return;
    }
    this.reveal(selector);
  }
}
