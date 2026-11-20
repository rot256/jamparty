// Live answers and open questions can pop up at their targets.
// Recent replies also appear in the comment list.

import { parsePlaceAnchor, placeElement } from "./answer-anchor";
import { answerViewport, drawAnswerLink, nearestSide, placeAnswer, SVG_NS } from "./answer-placement";
import type { Box, Placement } from "./answer-placement";
import { samePage } from "./capture";
import { dragBy, fitPosition } from "./drag";
import type { Point } from "./drag";
import { AGENT_TARGET_PREFIX, DISMISSED_KEY } from "./config";
import { MAX_ANSWERS } from "./generated/api";
import type { Answer, CommentOutcome, PlaceAnchor, Submission } from "./generated/api";
import type { PdfViewer } from "./pdf-viewer";
import type { Ui } from "./ui";

interface Shown {
  node: HTMLElement;
  wrapper: HTMLElement;
  marker: HTMLElement;
  link: SVGGElement;
  dropped?: Point;
  unavailable: HTMLElement;
  show: HTMLAnchorElement | null;
  target: () => Element | undefined;
  localAnchor: () => boolean;
  hasPlace: boolean;
  heightAt: (width: number) => number;
  stopTimer?: () => void;
}

interface PlaceTarget {
  place: PlaceAnchor;
  here: () => boolean;
  target: () => Element | undefined;
  detach: () => void;
}

function makeLink(): SVGGElement {
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

const REVEAL_KEY = "jamparty-answer-reveal";

/** How long a Show navigation may take to reach a page with the overlay. */
const REVEAL_TTL_MS = 30_000;

/** The answer a Show navigation in this tab asked to reveal, if recent: a
 * stale request (its page never loaded the overlay) must not reveal on an
 * unrelated page later. */
function takeReveal(): string | null {
  try {
    const stored = sessionStorage.getItem(REVEAL_KEY);
    sessionStorage.removeItem(REVEAL_KEY);
    const parsed: unknown = stored === null ? null : JSON.parse(stored);
    if (typeof parsed !== "object" || parsed === null) return null;
    const { id, at } = parsed as { id?: unknown; at?: unknown };
    if (typeof id !== "string" || typeof at !== "number") return null;
    const age = Date.now() - at;
    return age >= 0 && age <= REVEAL_TTL_MS ? id : null;
  } catch {
    return null;
  }
}

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
  private sections: HTMLElement | null = null;
  private readonly places = new Map<string, PlaceTarget>();
  private historyPreview: { id: string; node: HTMLElement } | undefined;
  private readonly recent = new Map<string, Answer>();
  onHistory: (answers: readonly Answer[]) => void = () => undefined;
  historyTarget: (id: string) => HTMLElement | null = () => null;
  private readonly dismissed = loadDismissed();
  private placementFrame: number | undefined;
  private timeoutSeconds = 0;
  private highlightTimer: number | undefined;
  private readonly geometry: ResizeObserver;
  private observedTargets = new Set<Element>();
  private pendingReveal = takeReveal();
  private readonly landingPath = location.pathname;

  constructor(
    private readonly ui: Ui,
    private readonly pick: (submission: Omit<Submission, "n">) => Promise<CommentOutcome>,
    private readonly pdf?: PdfViewer,
  ) {
    const schedule = (): void => { this.schedulePlacement(); };
    this.geometry = new ResizeObserver(schedule);
    this.geometry.observe(ui.list);
    this.geometry.observe(ui.bar);
    this.geometry.observe(ui.panel);
    this.geometry.observe(ui.agentGeneral);
    this.links.classList.add("agent-anchor-layer");
    this.links.setAttribute("aria-hidden", "true");
    ui.agentAnchored.prepend(this.links);
    window.visualViewport?.addEventListener("resize", schedule);
    window.visualViewport?.addEventListener("scroll", schedule);
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    window.addEventListener("load", schedule, true);
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
      const id = selector.slice(AGENT_TARGET_PREFIX.length);
      return this.node(id) ?? (this.dismissed.has(id) ? null : this.historyTarget(id));
    }
    return document.querySelector(selector);
  }

  /** A new or reconnected tab sees cached replies quietly. */
  sync(answers: readonly Answer[]): void {
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

  receive(answer: Answer): void {
    if (this.recent.has(answer.id)) return;
    this.recent.set(answer.id, answer);
    if (this.recent.size > MAX_ANSWERS) {
      const oldest = this.recent.keys().next();
      if (!oldest.done) this.recent.delete(oldest.value);
    }
    if (answer.live || answer.choices.length > 0) this.show(answer);
    this.publishHistory();
  }

  private publishHistory(): void {
    this.onHistory(Array.from(this.recent.values()).filter((answer) => answer.choices.length === 0));
    this.prunePlaces();
    this.schedulePlacement();
  }

  private renderBody(answer: Answer): HTMLDivElement {
    const body = document.createElement("div");
    body.className = "agent-body";
    // Rendered by the joiner with the restricted Markdown renderer.
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

  private placeTarget(answer: Answer): PlaceTarget | undefined {
    const existing = this.places.get(answer.id);
    if (existing !== undefined) return existing;
    const place = parsePlaceAnchor(answer.place);
    if (place === undefined) return undefined;
    // Session storage follows this tab across same-origin redirects. Keep the
    // landing path for this answer after the one-shot reveal has been consumed.
    const redirectedPath = answer.id === this.pendingReveal ? this.landingPath : undefined;
    const here = (): boolean => place.path === location.pathname || redirectedPath === location.pathname;
    const marker = place.type === "pdf" && this.pdf !== undefined && here()
      ? document.createElement("div") : undefined;
    let detach = (): void => undefined;
    if (marker !== undefined && place.type === "pdf" && this.pdf !== undefined) {
      marker.className = "pdf-answer-anchor";
      detach = this.pdf.trackAnswer(marker, place.pdf);
    }
    const target = (): Element | undefined => {
      if (!here()) return undefined;
      const element = place.type === "pdf" ? marker : placeElement(document, place.selector);
      return element?.isConnected && element.getClientRects().length > 0 ? element : undefined;
    };
    const entry = { place, here, target, detach };
    this.places.set(answer.id, entry);
    return entry;
  }

  private prunePlaces(): void {
    for (const [id, entry] of this.places) {
      if (this.recent.has(id) || this.shown.has(id)) continue;
      entry.detach();
      this.places.delete(id);
    }
  }

  private showLink(id: string, entry: PlaceTarget): HTMLAnchorElement {
    const link = document.createElement("a");
    link.className = "agent-show";
    link.textContent = "Show";
    link.href = new URL(entry.place.path, location.origin).href;
    link.addEventListener("click", (event) => {
      if (event.defaultPrevented || event.button !== 0 ||
          event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
      if (entry.here()) {
        event.preventDefault();
        const target = entry.target();
        if (target === undefined) this.ui.showToast("Target unavailable");
        else this.revealElement(target);
        return;
      }
      try {
        sessionStorage.setItem(REVEAL_KEY, JSON.stringify({ id, at: Date.now() }));
      } catch {
        // The link still navigates when session storage is unavailable.
      }
    });
    return link;
  }

  renderHistory(answer: Answer, node: HTMLElement): void {
    node.append(this.renderBody(answer));
    const entry = this.placeTarget(answer);
    if (entry === undefined) return;
    node.append(this.showLink(answer.id, entry));
    const preview = (): void => {
      this.historyPreview = { id: answer.id, node };
      this.schedulePlacement();
    };
    node.addEventListener("pointerenter", preview);
    node.addEventListener("focusin", preview);
    node.addEventListener("pointerleave", () => { this.schedulePlacement(); });
    node.addEventListener("focusout", () => { this.schedulePlacement(); });
  }

  private readonly links = document.createElementNS(SVG_NS, "svg");

  private show(answer: Answer): void {
    if (this.dismissed.has(answer.id) || this.shown.has(answer.id)) return;
    // The oldest go first (a Map keeps the order answers came in).
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

    const entry = this.placeTarget(answer);
    const selector = answer.anchor;
    const localAnchor = (): boolean => entry === undefined
      ? selector !== undefined && samePage(answer.page)
      : entry.here();
    const target = (): Element | undefined => {
      if (entry !== undefined) return entry.target();
      if (!localAnchor() || selector === undefined) return undefined;
      const element = this.findTarget(selector);
      if (!element || element === this.ui.host) return undefined;
      // A live answer may still be waiting for its first placement.
      if (selector.startsWith(AGENT_TARGET_PREFIX) &&
          this.node(selector.slice(AGENT_TARGET_PREFIX.length)) === element) return element;
      if (element.getClientRects().length === 0) return undefined;
      return element;
    };
    const activate = (): void => {
      if (!localAnchor()) return;
      const found = target();
      if (found === undefined) this.ui.showToast("Element is unavailable");
      else this.revealElement(found);
    };
    const show = entry !== undefined ? this.showLink(answer.id, entry)
      : selector === undefined ? null : document.createElement("a");
    if (show !== null) {
      if (entry === undefined) {
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
    wrapper.append(marker);
    const highlight = (active: boolean): void => {
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
    node.addEventListener("focusin", () => { highlight(true); });
    node.addEventListener("focusout", (event) => {
      highlight(hovered || (event.relatedTarget instanceof Node && node.contains(event.relatedTarget)));
    });
    // Anchored content is fixed for the lifetime of the answer. Measure
    // each width without changing the live card's overflow or scroll.
    const heights = new Map<number, number>();
    const heightAt = (width: number): number => {
      const cached = heights.get(width);
      if (cached !== undefined) return cached;
      const measure = node.cloneNode(true) as HTMLElement;
      measure.removeAttribute("data-message-id");
      measure.inert = true;
      measure.querySelector(".agent-timeout")?.remove();
      for (const detail of measure.querySelectorAll<HTMLElement>(".agent-show, .agent-unavailable")) {
        detail.hidden = true;
      }
      Object.assign(measure.style, {
        left: "-10000px", top: "0", width: `${width}px`, maxHeight: "none", visibility: "hidden",
      });
      this.ui.agentAnchored.append(measure);
      const height = measure.getBoundingClientRect().height;
      measure.remove();
      heights.set(width, height);
      return height;
    };
    const shown: Shown = {
      node, wrapper, marker, link, unavailable, show, target, localAnchor, hasPlace: entry !== undefined, heightAt,
    };
    dragBy(header, node, (point) => {
      if (shown.dropped === undefined) {
        const rect = node.getBoundingClientRect();
        node.style.width = `${rect.width}px`;
        node.classList.add("manually-placed");
      }
      shown.dropped = point;
      this.positionMoved(shown);
      this.schedulePlacement();
    });
    this.shown.set(answer.id, shown);
    this.schedulePlacement();
  }

  /** The share resolved this question; no session dismissal is needed. */
  remove(id: string): void {
    if (this.recent.get(id)?.choices.length) this.recent.delete(id);
    const shown = this.shown.get(id);
    shown?.stopTimer?.();
    shown?.node.remove();
    shown?.wrapper.remove();
    shown?.link.remove();
    this.shown.delete(id);
    this.prunePlaces();
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
    let remaining = this.timeoutSeconds * 1000;
    let started: number | undefined;
    let timer: number | undefined;
    let hovering = node.matches(":hover");
    let visible = false;
    const update = (): void => {
      if (stopped) return;
      const paused = !visible || document.hidden || hovering || node.contains(this.ui.shadow.activeElement);
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

  private persistDismissed(): void {
    try {
      sessionStorage.setItem(DISMISSED_KEY, JSON.stringify(Array.from(this.dismissed)));
    } catch {
      // Dismissal remains active for this document if storage is unavailable.
    }
  }

  private findTarget(selector: string): Element | null {
    try {
      return this.targetFor(selector);
    } catch {
      return null;
    }
  }

  schedulePlacement(): void {
    if (this.placementFrame !== undefined) return;
    this.placementFrame = requestAnimationFrame(() => { this.place(); });
  }

  private place(): void {
    this.placementFrame = undefined;
    const { agentAnchored, agentGeneral } = this.ui;
    const anchored = new Map<Shown, Element>();
    const cards = new Map<Element, Shown>();
    let hovered: Element | undefined;
    const fixed = new Map<Shown, Box>();
    for (const entry of this.shown.values()) {
      if (entry.dropped !== undefined) fixed.set(entry, this.positionMoved(entry));
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
      if (target !== undefined) anchored.set(entry, target);
      else this.placeGeneral(entry, local);
    }

    const preview = this.historyPreview;
    if (preview !== undefined) {
      if (preview.node.getClientRects().length > 0 &&
          (preview.node.matches(":hover") || preview.node.contains(this.ui.shadow.activeElement))) {
        hovered = this.places.get(preview.id)?.target();
      } else {
        this.historyPreview = undefined;
      }
    }
    if (this.pendingReveal !== null) {
      const target = this.places.get(this.pendingReveal)?.target();
      if (target !== undefined) {
        this.pendingReveal = null;
        this.revealElement(target);
      }
    }

    if (hovered !== undefined) this.outline(hovered);
    else if (this.highlightTimer === undefined) this.ui.messageHighlight.style.display = "none";

    // The PDF viewer adds its sections panel after Answers is constructed.
    if (this.pdf !== undefined && this.sections === null) {
      this.sections = this.ui.shadow.querySelector("#pdf-sections");
      if (this.sections !== null) this.geometry.observe(this.sections);
    }
    const sectionsHeight = this.sections?.offsetHeight ?? 0;
    const sectionsOffset = `${sectionsHeight === 0 ? 0 : sectionsHeight + 14}px`;
    if (agentGeneral.style.getPropertyValue("--sections-offset") !== sectionsOffset) {
      agentGeneral.style.setProperty("--sections-offset", sectionsOffset);
    }
    const obstacles = [
      this.ui.bar,
      this.ui.list,
      this.ui.panel,
      ...(this.sections === null ? [] : [this.sections]),
      ...(this.pdf === undefined ? [] : document.querySelectorAll("#pdf-feedback > :not(:empty)")),
    ];
    const occupied: Box[] = obstacles.filter((node) => node.getClientRects().length > 0 &&
      getComputedStyle(node).visibility === "visible").map((node) => node.getBoundingClientRect());
    occupied.push(...fixed.values());
    const targets = new Set(anchored.values());
    for (const target of this.observedTargets) if (!targets.has(target)) this.geometry.unobserve(target);
    for (const target of targets) if (!this.observedTargets.has(target)) this.geometry.observe(target);
    this.observedTargets = targets;

    const view = answerViewport();
    const pdfViewer = document.documentElement.hasAttribute("data-jamparty-pdf");
    const targetBoxes = new Map<Element, Box>();
    const pages = new Map<Element, Box>();
    for (const target of targets) {
      const parent = cards.get(target);
      if (parent !== undefined && anchored.has(parent)) continue;
      targetBoxes.set(target, target.getBoundingClientRect());
      // Only the joiner's PDF viewer has page sheets; a shared HTML page may
      // use the same class name for anything.
      const page = pdfViewer ? target.closest(".pdf-page") : null;
      if (page !== null) pages.set(target, page.getBoundingClientRect());
    }
    const general = new Map(
      Array.from(agentGeneral.children)
        .filter((node) => !node.classList.contains("manually-placed"))
        .map((node) => [node, node.getBoundingClientRect()]),
    );
    const pending = new Set(anchored.keys());
    const placements = new Map<Shown, { rect: Box; placement: Placement }>();
    const solve = (entry: Shown): void => {
      if (!pending.delete(entry)) return;
      const target = anchored.get(entry);
      if (target === undefined) return;
      const parent = cards.get(target);
      if (parent !== undefined) solve(parent);
      const rect = targetBoxes.get(target);
      const previous = general.get(entry.node);
      general.delete(entry.node);
      const moved = fixed.get(entry);
      const placement = rect === undefined ? undefined : moved !== undefined
        ? { box: moved, side: nearestSide(rect, moved), maxHeight: view.bottom - view.top }
        : placeAnswer(entry.heightAt, view, rect, pages.get(target), [
        ...occupied,
        ...general.values(),
        ...Array.from(targetBoxes.values()).filter((box) => box !== rect),
      ]);
      if (placement === undefined || rect === undefined) {
        this.placeGeneral(entry, false);
        if (previous === undefined) {
          // Inserting into the general stack can shift its other cards.
          for (const node of general.keys()) {
            const box = node.getBoundingClientRect();
            general.set(node, box);
            if (targetBoxes.has(node)) targetBoxes.set(node, box);
          }
          this.schedulePlacement();
        }
        const box = previous ?? entry.node.getBoundingClientRect();
        general.set(entry.node, box);
        if (targets.has(entry.node)) targetBoxes.set(entry.node, box);
        return;
      }
      // Border-box sizing gives dependents their parent's final geometry
      // before we apply the styles to the live cards.
      if (moved === undefined) occupied.push(placement.box);
      if (targets.has(entry.node)) targetBoxes.set(entry.node, placement.box);
      placements.set(entry, { rect, placement });
    };
    const ordered = Array.from(anchored).sort(([, a], [, b]) =>
      (targetBoxes.get(a)?.top ?? Infinity) - (targetBoxes.get(b)?.top ?? Infinity));
    for (const [entry] of ordered) solve(entry);
    for (const [entry, { rect, placement }] of placements) {
      entry.unavailable.hidden = true;
      if (entry.show !== null) entry.show.hidden = true;
      if (entry.node.parentElement !== entry.wrapper && entry.dropped === undefined) {
        if (entry.node.parentElement === agentGeneral) this.schedulePlacement();
        entry.wrapper.prepend(entry.node);
      }
      if (entry.wrapper.parentElement !== agentAnchored) agentAnchored.append(entry.wrapper);
      const { box, maxHeight } = placement;
      if (entry.dropped === undefined) {
        entry.node.style.left = `${box.left}px`;
        entry.node.style.top = `${box.top}px`;
        entry.node.style.width = `${box.right - box.left}px`;
        entry.node.style.maxHeight = `${maxHeight}px`;
      }
      if (entry.link.parentElement === null) this.links.append(entry.link);
      drawAnswerLink(entry.marker, entry.link, view, rect, placement);
    }
    agentGeneral.classList.toggle("scrollable", agentGeneral.scrollHeight > agentGeneral.clientHeight);
    for (const entry of this.shown.values()) {
      if (entry.stopTimer === undefined) this.startTimer(entry);
    }
  }

  private positionMoved(entry: Shown): Box {
    const { node, dropped } = entry;
    if (dropped !== undefined) {
      const view = answerViewport();
      node.style.maxWidth = `${view.right - view.left}px`;
      node.style.maxHeight = `${view.bottom - view.top}px`;
      const point = fitPosition(dropped, node.offsetWidth, node.offsetHeight);
      node.style.left = `${point.x}px`;
      node.style.top = `${point.y}px`;
    }
    return node.getBoundingClientRect();
  }

  private placeGeneral(entry: Shown, unavailable: boolean): void {
    if (entry.unavailable.hidden === unavailable) entry.unavailable.hidden = !unavailable;
    if (entry.show !== null) {
      const hidden = unavailable || (!entry.hasPlace && !entry.localAnchor());
      if (entry.show.hidden !== hidden) entry.show.hidden = hidden;
    }
    entry.link.remove();
    if (entry.dropped !== undefined) {
      entry.marker.hidden = true;
      return;
    }
    if (entry.node.parentElement === this.ui.agentGeneral) return;
    this.ui.agentGeneral.append(entry.node);
    entry.wrapper.remove();
    for (const property of ["left", "top", "width", "max-height"]) {
      entry.node.style.removeProperty(property);
    }
  }

  private outline(target: Element): void {
    const { messageHighlight } = this.ui;
    const rect = target.getBoundingClientRect();
    messageHighlight.style.display = "block";
    messageHighlight.style.left = `${rect.left}px`;
    messageHighlight.style.top = `${rect.top}px`;
    messageHighlight.style.width = `${rect.width}px`;
    messageHighlight.style.height = `${rect.height}px`;
  }

  private showHighlight(target: Element): void {
    this.outline(target);
    if (this.highlightTimer !== undefined) clearTimeout(this.highlightTimer);
    this.highlightTimer = window.setTimeout(() => {
      this.highlightTimer = undefined;
      this.schedulePlacement();
    }, 1800);
  }

  /** Scrolls to the element `selector` names and highlights it briefly. */
  reveal(selector: string): void {
    const target = this.findTarget(selector);
    if (!target || target.getClientRects().length === 0) {
      this.ui.showToast("Element is unavailable");
      return;
    }
    this.revealElement(target);
  }

  private revealElement(target: Element): void {
    target.scrollIntoView({ block: "center", inline: "nearest" });
    requestAnimationFrame(() => {
      if (target.isConnected) this.showHighlight(target);
    });
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
