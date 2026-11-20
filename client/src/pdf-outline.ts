import type { PDFDocumentProxy, PageViewport } from "pdfjs-dist";

import type { Ui } from "./ui";

const MAX_ENTRIES = 2000;
const MAX_DEPTH = 8;
const MAX_TITLE = 256;

type RefProxy = Parameters<PDFDocumentProxy["getPageIndex"]>[0];
type RawEntry = Awaited<ReturnType<PDFDocumentProxy["getOutline"]>>[number];

export interface PdfDestination {
  page: number;
  left?: number | undefined;
  top?: number | undefined;
  right?: number | undefined;
  bottom?: number | undefined;
}

interface Entry {
  key: string;
  title: string;
  raw: RawEntry["dest"];
  parent: Entry | undefined;
  children: Entry[];
  destination?: PdfDestination | undefined;
  button?: HTMLButtonElement;
}

function isRef(value: unknown): value is RefProxy {
  return typeof value === "object" && value !== null && "num" in value && "gen" in value &&
    Number.isSafeInteger(value.num) && Number.isSafeInteger(value.gen);
}

function coordinate(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

async function resolveDestination(
  pdf: PDFDocumentProxy, raw: RawEntry["dest"],
): Promise<PdfDestination | undefined> {
  const dest: readonly unknown[] | null = typeof raw === "string" ? await pdf.getDestination(raw) : raw;
  if (dest === null) return;
  const [target, kind, a, b, c, d] = dest;
  const index = typeof target === "number" ? target
    : isRef(target) ? await pdf.getPageIndex(target) : -1;
  if (!Number.isInteger(index) || index < 0 || index >= pdf.numPages) return;
  const page = index + 1;
  if (typeof kind !== "object" || kind === null || !("name" in kind)) return;
  switch (kind.name) {
    case "XYZ": return { page, left: coordinate(a), top: coordinate(b) };
    case "FitH": case "FitBH": return { page, top: coordinate(a) };
    case "FitV": case "FitBV": return { page, left: coordinate(a) };
    case "FitR": return {
      page, left: coordinate(a), top: coordinate(d), right: coordinate(c), bottom: coordinate(b),
    };
    case "Fit": case "FitB": return { page };
    default: return undefined;
  }
}

export function destinationY(viewport: PageViewport, destination: PdfDestination): number {
  const { left, top, right, bottom } = destination;
  const [startX, startY] = viewport.convertToPdfPoint(0, 0) as [number, number];
  const [, y] = viewport.convertToViewportPoint(left ?? startX, top ?? startY) as [number, number];
  const [, otherY] = viewport.convertToViewportPoint(right ?? left ?? startX, bottom ?? top ?? startY) as [number, number];
  return Math.max(0, Math.min(1, Math.min(y, otherY) / viewport.height));
}

export class PdfOutline {
  private readonly toggle = document.createElement("button");
  private readonly panel = document.createElement("section");
  private readonly content = document.createElement("div");
  private readonly expanded = new Set<string>();
  private roots: Entry[] = [];
  private entries: Entry[] = [];
  private active: HTMLButtonElement | undefined;
  private pdf: PDFDocumentProxy | undefined;
  private started = false;
  private navigation = 0;

  constructor(
    ui: Ui,
    private readonly jump: (page: number, y: number) => void,
    private readonly updateCurrent: () => void,
    onOpen: () => void,
  ) {
    this.toggle.id = "pdf-outline";
    this.toggle.type = "button";
    this.toggle.hidden = true;
    this.toggle.title = "PDF sections";
    this.toggle.setAttribute("aria-label", "PDF sections");
    this.toggle.setAttribute("aria-controls", "pdf-sections");
    this.toggle.setAttribute("aria-expanded", "false");
    this.toggle.innerHTML = '<svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M3 4h10M6 8h7M6 12h7"/></svg>';
    ui.edit.after(this.toggle);

    this.panel.id = "pdf-sections";
    this.panel.hidden = true;
    this.panel.tabIndex = -1;
    this.panel.setAttribute("aria-labelledby", "pdf-sections-heading");
    const heading = document.createElement("h2");
    heading.id = "pdf-sections-heading";
    heading.textContent = "Sections";
    this.panel.append(heading, this.content);
    ui.bar.after(this.panel);

    this.toggle.addEventListener("click", () => {
      if (!this.panel.hidden) {
        this.close();
        return;
      }
      onOpen();
      this.panel.hidden = false;
      this.toggle.setAttribute("aria-expanded", "true");
      this.panel.focus({ preventScroll: true });
      void this.loadEntries();
      this.updateCurrent();
    });
    document.addEventListener("pointerdown", (event) => {
      const path = event.composedPath();
      if (!path.includes(this.panel) && !path.includes(this.toggle)) this.close();
    }, true);
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || event.defaultPrevented || this.panel.hidden) return;
      event.preventDefault();
      this.close();
      this.toggle.focus({ preventScroll: true });
    }, true);
  }

  close(): void {
    if (this.panel.hidden) return;
    this.navigation++;
    this.panel.hidden = true;
    this.toggle.setAttribute("aria-expanded", "false");
  }

  setDocument(pdf: PDFDocumentProxy): void {
    this.pdf = pdf;
    this.roots = [];
    this.entries = [];
    this.active = undefined;
    this.started = false;
    this.content.replaceChildren();
    void this.readOutline(pdf);
  }

  update(page: number, y: number, getY: (destination: PdfDestination) => number | undefined): void {
    if (this.panel.hidden) return;
    let current: Entry | undefined;
    let bestPage = 0;
    let bestY = -Infinity;
    for (const entry of this.entries) {
      const target = entry.destination;
      if (target === undefined || target.page < bestPage || target.page > page) continue;
      const offset = getY(target);
      if (target.page === page && (offset ?? 0) > y) continue;
      // Earlier pages may be unloaded. Raw PDF top coordinates grow upwards.
      const order = offset ?? -(target.top ?? Infinity);
      if (target.page === bestPage && order < bestY) continue;
      current = entry;
      bestPage = target.page;
      bestY = order;
    }
    for (let parent = current?.parent; parent !== undefined; parent = parent.parent) {
      if (!this.expanded.has(parent.key)) current = parent;
    }
    if (current?.button === this.active) return;
    this.active?.removeAttribute("aria-current");
    this.active = current?.button;
    this.active?.setAttribute("aria-current", "location");
  }

  private async readOutline(pdf: PDFDocumentProxy): Promise<void> {
    const entries: Entry[] = [];
    const collect = (items: readonly RawEntry[], parent?: Entry, depth = 1): Entry[] => {
      const children: Entry[] = [];
      if (depth > MAX_DEPTH) return children;
      const occurrences = new Map<string, number>();
      for (const item of items) {
        if (entries.length === MAX_ENTRIES) break;
        const title = item.title.slice(0, MAX_TITLE).trim() || "Untitled section";
        const occurrence = occurrences.get(title) ?? 0;
        occurrences.set(title, occurrence + 1);
        const key = `${parent?.key ?? ""}/${JSON.stringify(title)}/${occurrence}`;
        const entry: Entry = { key, title, raw: item.dest, parent, children: [] };
        children.push(entry);
        entries.push(entry);
        entry.children = collect(item.items as RawEntry[], entry, depth + 1);
      }
      return children;
    };

    const outline: RawEntry[] | null = await pdf.getOutline().catch(() => null);
    if (this.pdf !== pdf) return;
    this.roots = collect(outline ?? []);
    this.entries = entries;
    this.toggle.hidden = entries.length === 0;
    const keys = new Set(entries.filter((entry) => entry.children.length > 0).map(({ key }) => key));
    for (const key of this.expanded) {
      if (!keys.has(key)) this.expanded.delete(key);
    }
    if (this.toggle.hidden) this.close();
    else if (!this.panel.hidden) void this.loadEntries();
  }

  private async loadEntries(): Promise<void> {
    const pdf = this.pdf;
    if (pdf === undefined || this.entries.length === 0 || this.started) return;
    this.started = true;
    const entries = this.entries;
    for (let i = 0; i < entries.length; i += 8) {
      if (this.pdf !== pdf) return;
      await Promise.all(entries.slice(i, i + 8).map(async (entry) => {
        entry.destination = await resolveDestination(pdf, entry.raw).catch(() => undefined);
      }));
    }
    if (this.pdf !== pdf) return;
    this.content.replaceChildren(this.renderList(this.roots));
    this.updateCurrent();
  }

  private renderList(entries: readonly Entry[]): HTMLUListElement {
    const list = document.createElement("ul");
    for (const entry of entries) {
      const item = document.createElement("li");
      const row = document.createElement("div");
      row.className = "pdf-section-row";
      item.append(row);
      if (entry.children.length > 0) {
        const children = this.renderList(entry.children);
        const expand = document.createElement("button");
        expand.type = "button";
        expand.className = "pdf-section-disclosure";
        expand.setAttribute("aria-label", `Subsections of ${entry.title}`);
        const show = (open: boolean): void => {
          expand.setAttribute("aria-expanded", String(open));
          expand.textContent = open ? "▾" : "▸";
          children.hidden = !open;
        };
        show(this.expanded.has(entry.key));
        expand.addEventListener("click", () => {
          if (this.expanded.has(entry.key)) this.expanded.delete(entry.key);
          else this.expanded.add(entry.key);
          show(this.expanded.has(entry.key));
          this.updateCurrent();
        });
        row.append(expand);
        item.append(children);
      }
      const button = document.createElement("button");
      button.type = "button";
      button.className = "pdf-section-link";
      button.textContent = entry.title;
      const destination = entry.destination;
      button.disabled = destination === undefined;
      if (destination !== undefined) {
        button.addEventListener("click", () => { void this.navigate(destination); });
      }
      entry.button = button;
      row.append(button);
      list.append(item);
    }
    return list;
  }

  private async navigate(destination: PdfDestination): Promise<void> {
    const pdf = this.pdf;
    if (pdf === undefined) return;
    const navigation = ++this.navigation;
    try {
      const page = await pdf.getPage(destination.page);
      if (this.pdf !== pdf || this.navigation !== navigation || this.panel.hidden) return;
      const y = destinationY(page.getViewport({ scale: 1 }), destination);
      this.close();
      this.toggle.focus({ preventScroll: true });
      this.jump(destination.page, y);
    } catch {
      // The entry or document may have gone away during navigation.
    }
  }
}
