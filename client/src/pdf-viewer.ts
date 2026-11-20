import { getDocument, GlobalWorkerOptions } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PDFDocumentLoadingTask } from "pdfjs-dist";

import { samePage, selectedTextFor } from "./capture";
import type { CommentDialog } from "./dialog";
import type { OutboxEntry } from "./outbox";
import type { PdfAnchor } from "./pdf-anchor";
import { PdfPage, placeRectangle, rectangle } from "./pdf-page";
import { findReadingPosition } from "./pdf-reading-position";
import type { Ui } from "./ui";

interface ViewPosition {
  page: number;
  y: number;
  x: number;
  offset?: number;
}

interface Drag {
  id: number;
  page: PdfPage;
  x: number;
  y: number;
  outline: HTMLDivElement;
  moved: boolean;
}

const DRAG_THRESHOLD = 3;

const MAX_PDF_BYTES = 128 * 1024 * 1024;
const MAX_PDF_PAGES = 1000;

class PdfLimitError extends Error {}

async function readPdf(response: Response): Promise<Uint8Array> {
  const limitMessage = "This PDF exceeds the 128 MiB viewer limit.";
  if (Number(response.headers.get("content-length")) > MAX_PDF_BYTES) {
    void response.body?.cancel().catch(() => {});
    throw new PdfLimitError(limitMessage);
  }
  if (response.body === null) throw new Error("Missing PDF body");
  const reader = response.body.getReader();
  let data: Uint8Array<ArrayBuffer> = new Uint8Array(0);
  let size = 0;
  try {
    for (let chunk = await reader.read(); !chunk.done; chunk = await reader.read()) {
      const { value } = chunk;
      const end = size + value.byteLength;
      if (end > MAX_PDF_BYTES) {
        void reader.cancel().catch(() => {});
        throw new PdfLimitError(limitMessage);
      }
      if (end > data.byteLength) {
        const capacity = Math.min(MAX_PDF_BYTES, Math.max(end, data.byteLength * 2, 64 * 1024));
        const grown = new Uint8Array(capacity);
        grown.set(data);
        data = grown;
      }
      data.set(value, size);
      size = end;
    }
  } finally {
    reader.releaseLock();
  }
  return data.subarray(0, size);
}

export class PdfViewer {
  private readonly container = document.querySelector<HTMLElement>("#pdf-pages");
  private readonly pageInput = document.querySelector<HTMLInputElement>("#pdf-page");
  private readonly zoomInput = document.querySelector<HTMLSelectElement>("#pdf-zoom");
  private readonly status = document.querySelector<HTMLElement>("#pdf-status");
  private readonly count = document.querySelector<HTMLElement>("#pdf-count");
  private readonly pages = new Map<Element, PdfPage>();
  private readonly byNumber = new Map<number, PdfPage>();
  private readonly visible = new Set<PdfPage>();
  private readonly observer = new IntersectionObserver((entries) => {
    for (const entry of entries) {
      const page = this.pages.get(entry.target);
      if (page === undefined) continue;
      if (entry.isIntersecting) {
        this.visible.add(page);
        page.setVisible(true);
      } else {
        this.visible.delete(page);
        page.setVisible(false);
      }
    }
  }, { rootMargin: "800px 0px" });
  private loading: PDFDocumentLoadingTask | null = null;
  private document: PDFDocumentLoadingTask | null = null;
  private requestedRevision = 0;
  private displayedRevision: number | null = null;
  private request: AbortController | null = null;
  private zoom: number | "fit" = "fit";
  private preview: OutboxEntry | null = null;
  private comments: readonly OutboxEntry[] = [];
  private dialog: CommentDialog | null = null;
  private drawing = false;
  private drag: Drag | null = null;
  private readonly hover = document.createElement("div");
  private hoverPoint: { page: PdfPage; x: number; y: number } | undefined;
  private hoverRequest: Promise<void> | undefined;
  private gesture = 0;
  private draftMarker: { anchor: PdfAnchor; node: HTMLDivElement } | null = null;
  private readingRestore: { position: ViewPosition; x: number; y: number } | undefined;
  private scrollFrame: number | undefined;

  constructor() {
    GlobalWorkerOptions.workerSrc = "/_jamparty/pdf-worker.js";
    this.pageInput?.addEventListener("change", () => {
      const page = this.pageInput;
      if (page === null) return;
      const number = Math.max(1, Math.min(this.pages.size, Math.trunc(page.valueAsNumber) || 1));
      this.byNumber.get(number)?.element.scrollIntoView({ block: "start", inline: "nearest" });
      page.valueAsNumber = number;
    });
    this.zoomInput?.addEventListener("change", () => {
      const value = this.zoomInput?.value;
      const position = this.position();
      this.readingRestore = undefined;
      this.zoom = value === "fit" ? value : Number(value);
      this.resize(position);
    });
    window.addEventListener("resize", () => {
      this.readingRestore = undefined;
      if (this.zoom === "fit") this.resize(this.position());
    });
    window.addEventListener("scroll", () => {
      this.activeReadingRestore();
      this.scrollFrame ??= requestAnimationFrame(() => {
        this.scrollFrame = undefined;
        if (this.pageInput !== null && document.activeElement !== this.pageInput) {
          this.pageInput.valueAsNumber = this.position().page;
        }
      });
    }, { passive: true });
  }

  requestRevision(revision: number): void {
    if (revision <= this.requestedRevision) return;
    this.requestedRevision = revision;
    this.suspend();
  }

  suspend(): void {
    this.request?.abort();
    this.request = null;
    if (this.loading !== null) void this.loading.destroy();
    this.loading = null;
  }

  ensureLoaded(): void {
    if (this.request !== null || this.displayedRevision === this.requestedRevision) return;
    const request = new AbortController();
    this.request = request;
    if (this.status !== null) {
      this.status.textContent = this.pages.size === 0 ? "Loading PDF…" : "Updating PDF…";
    }
    void this.load(request, this.requestedRevision);
  }

  private async load(request: AbortController, revision: number): Promise<void> {
    let loading: PDFDocumentLoadingTask | null = null;
    try {
      const response = await fetch(location.href, {
        cache: "no-store", redirect: "error", signal: request.signal,
      });
      if (!response.ok) throw new Error("PDF request failed");
      const data = await readPdf(response);
      request.signal.throwIfAborted();
      const options = {
        data,
        // PDF.js 6 has removed eval-based font rendering.
        isEvalSupported: false,
        useWasm: false,
        useWorkerFetch: false,
        cMapUrl: "/_jamparty/pdf-cmaps/",
        cMapPacked: true,
        enableXfa: false,
        useSystemFonts: true,
        maxImageSize: 16_000_000,
      };
      loading = getDocument(options);
      this.loading = loading;
      const pdf = await loading.promise;
      if (pdf.numPages > MAX_PDF_PAGES) {
        throw new PdfLimitError(`This PDF exceeds the ${MAX_PDF_PAGES}-page viewer limit.`);
      }
      const first = await pdf.getPage(1);
      request.signal.throwIfAborted();
      const pages = Array.from({ length: pdf.numPages }, (_, index) => {
        const number = index + 1;
        return new PdfPage(number, first, () => pdf.getPage(number), () => {
          const page = this.byNumber.get(number);
          if (page === undefined || this.document !== loading) return;
          const position = this.activeReadingRestore() ?? this.position();
          page.resize(this.zoom, this.availableWidth());
          this.drawComments();
          page.render();
          this.restorePosition(position);
        }, () => {
          if (this.hoverPoint?.page.number === number && this.document === loading) this.updateHover();
        });
      });
      const position = this.document === null ? this.position()
        : await this.rebuildPosition(pages, request.signal);
      request.signal.throwIfAborted();
      const previous = this.document;
      this.loading = null;
      this.document = loading;
      this.replacePages(pages, position);
      this.displayedRevision = revision;
      if (previous !== null) void previous.destroy();
      if (this.status !== null) this.status.textContent = "";
    } catch (error) {
      if (request.signal.aborted) return;
      this.loading = null;
      if (loading !== null && loading !== this.document) void loading.destroy();
      if (this.status !== null) {
        this.status.textContent = error instanceof PdfLimitError ? error.message
          : this.pages.size === 0 ? "Could not open this PDF." : "Could not open the updated PDF.";
      }
    } finally {
      if (this.request === request) this.request = null;
    }
  }

  private async rebuildPosition(pages: PdfPage[], signal: AbortSignal): Promise<ViewPosition> {
    const position = this.position();
    const page = this.byNumber.get(position.page);
    if (page === undefined) return position;

    const scroll = new AbortController();
    const x = window.scrollX;
    const y = window.scrollY;
    const moved = (): boolean => window.scrollX !== x || window.scrollY !== y;
    const onScroll = (): void => { if (moved()) scroll.abort(); };
    window.addEventListener("scroll", onScroll, { passive: true });
    const match = await findReadingPosition(page, this.scrollEdge(), pages,
      AbortSignal.any([signal, scroll.signal]))
      .catch(() => undefined)
      .finally(() => { window.removeEventListener("scroll", onScroll); });
    if (scroll.signal.aborted || moved()) return this.position();
    return match === undefined ? position : { ...match, x: position.x };
  }

  private replacePages(pages: PdfPage[], position: ViewPosition): void {
    this.cancelDrag();
    this.clearHover();
    this.gesture++;
    this.observer.disconnect();
    this.observer.takeRecords();
    for (const page of this.pages.values()) page.setVisible(false);
    this.pages.clear();
    this.byNumber.clear();
    this.visible.clear();
    for (const page of pages) {
      this.pages.set(page.element, page);
      this.byNumber.set(page.number, page);
    }
    this.readingRestore = undefined;
    this.container?.replaceChildren(...this.pages.keys());
    this.resize(position);
    for (const page of this.pages.keys()) this.observer.observe(page);
    if (this.count !== null) this.count.textContent = `of ${pages.length}`;
    if (this.pageInput !== null) {
      this.pageInput.max = String(pages.length);
      this.pageInput.valueAsNumber = Math.min(position.page, pages.length);
      this.pageInput.disabled = pages.length === 0;
    }
  }

  private position(): ViewPosition {
    const top = this.scrollEdge();
    // Page boxes are laid out from top to bottom with a fixed gap.
    let low = 1;
    let high = this.pages.size;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      const bottom = this.byNumber.get(middle)?.element.getBoundingClientRect().bottom ?? 0;
      if (bottom <= top) low = middle + 1;
      else high = middle;
    }
    const rect = this.byNumber.get(low)?.element.getBoundingClientRect();
    if (rect === undefined) return { page: 1, y: 0, x: 0 };
    return {
      page: low,
      y: (top - rect.top) / rect.height,
      x: Math.max(0, -rect.left / rect.width),
    };
  }

  private scrollEdge(): number {
    return (document.querySelector("nav")?.getBoundingClientRect().bottom ?? 0) + 12;
  }

  private availableWidth(): number {
    return Math.max(1, Math.min(960, document.documentElement.clientWidth - 32));
  }

  private resize(position: ViewPosition): void {
    this.clearHover();
    this.gesture++;
    const available = this.availableWidth();
    for (const page of this.pages.values()) page.resize(this.zoom, available);
    this.drawComments();
    for (const page of this.visible) page.render();
    this.restorePosition(position);
  }

  private activeReadingRestore(): ViewPosition | undefined {
    const restore = this.readingRestore;
    if (restore?.x !== window.scrollX || restore.y !== window.scrollY) {
      this.readingRestore = undefined;
      return undefined;
    }
    return restore.position;
  }

  private restorePosition(position: ViewPosition): void {
    const rect = this.byNumber.get(Math.min(position.page, this.pages.size))
      ?.element.getBoundingClientRect();
    if (rect !== undefined) {
      window.scrollBy(position.x === 0 ? -window.scrollX : rect.left + position.x * rect.width,
        rect.top + position.y * rect.height - this.scrollEdge() - (position.offset ?? 0));
      if (position.offset !== undefined) {
        // Unmeasured pages can resize above the matched line after the swap.
        // Keep targeting that line until the reader moves.
        this.readingRestore = { position, x: window.scrollX, y: window.scrollY };
      }
    }
  }

  showComments(entries: readonly OutboxEntry[]): void {
    this.comments = entries;
    this.drawComments();
  }

  previewComment(entry: OutboxEntry | null): void {
    if (entry === this.preview) return;
    this.previewNode(this.preview)?.classList.remove("pdf-comment");
    this.preview = entry;
    this.previewNode(entry)?.classList.add("pdf-comment");
  }

  private previewNode(entry: OutboxEntry | null): Element | null {
    if (entry?.draft.pdf === undefined || !samePage(entry.draft.page)) return null;
    const node = document.getElementById(entry.draft.selector.slice(1));
    return node?.matches(".pdf-comment-anchor") ? node : null;
  }

  private drawComments(): void {
    for (const page of this.pages.values()) page.marks.replaceChildren();
    for (const entry of this.comments) {
      const { draft } = entry;
      if (draft.pdf === undefined || !samePage(draft.page) ||
          !/^#jamparty-pdf-anchor-[\da-f-]+$/.test(draft.selector)) continue;
      const node = document.createElement("div");
      // Answer placement uses the anchor geometry even without a preview.
      node.className = "pdf-comment-anchor";
      node.classList.toggle("pdf-comment", entry === this.preview);
      node.id = draft.selector.slice(1);
      this.byNumber.get(draft.pdf.page)?.mark(node, draft.pdf);
    }
    if (this.draftMarker !== null && this.dialog?.open) {
      const { anchor, node } = this.draftMarker;
      this.byNumber.get(anchor.page)?.mark(node, anchor);
    }
  }

  bind(ui: Ui, dialog: CommentDialog): void {
    this.dialog = dialog;
    this.hover.className = "pdf-block-hover";
    ui.edit.setAttribute("aria-label", "Select a PDF block or draw a box");
    ui.edit.title = "Select a PDF block or draw a box";
    const setDrawing = (on: boolean): void => {
      this.drawing = on;
      ui.edit.setAttribute("aria-pressed", String(on));
      this.container?.classList.toggle("pdf-drawing", on);
      this.gesture++;
      if (!on) {
        this.cancelDrag();
        this.clearHover();
      }
    };
    ui.edit.addEventListener("click", () => { setDrawing(!this.drawing); });
    dialog.onOpen = ({ pdf: anchor }) => {
      this.clearHover();
      if (anchor === undefined) return;
      const node = document.createElement("div");
      node.className = "pdf-draft";
      this.draftMarker = { anchor, node };
      this.byNumber.get(anchor.page)?.mark(node, anchor);
    };
    dialog.onClose = () => {
      this.draftMarker?.node.remove();
      this.draftMarker = null;
      this.clearHover();
    };
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !this.drawing || dialog.open) return;
      event.preventDefault();
      setDrawing(false);
    }, true);
    this.container?.addEventListener("pointerdown", (event) => {
      if (!this.drawing || dialog.open || !event.isPrimary || event.button !== 0) return;
      const page = this.pageFor(event.target);
      if (page === undefined) return;
      event.preventDefault();
      this.gesture++;
      this.clearHover();
      page.element.setPointerCapture(event.pointerId);
      const outline = document.createElement("div");
      outline.className = "pdf-drag";
      page.element.append(outline);
      this.drag = { id: event.pointerId, page, x: event.clientX, y: event.clientY, outline, moved: false };
    });
    this.container?.addEventListener("pointermove", (event) => {
      if (this.drag !== null) this.moveDrag(event);
      else if (this.drawing && !dialog.open && event.isPrimary && event.pointerType !== "touch") {
        const page = this.pageFor(event.target);
        if (page === undefined) this.clearHover();
        else {
          this.hoverPoint = { page, x: event.clientX, y: event.clientY };
          this.updateHover();
        }
      }
    });
    this.container?.addEventListener("pointerleave", () => { this.clearHover(); });
    window.addEventListener("blur", () => { this.clearHover(); });
    window.addEventListener("scroll", () => {
      this.clearHover();
      this.gesture++;
    }, { passive: true });
    this.container?.addEventListener("lostpointercapture", (event) => {
      if (this.drag?.id === event.pointerId) this.cancelDrag();
    });
    this.container?.addEventListener("pointerup", (event) => {
      const drag = this.drag;
      if (drag === null || drag.id !== event.pointerId) return;
      this.moveDrag(event);
      this.cancelDrag();
      if (!drag.moved) {
        this.selectBlock(drag.page, event.clientX, event.clientY);
        return;
      }
      const anchor = drag.page.anchor(rectangle(drag.x, drag.y, event.clientX, event.clientY));
      if (anchor !== undefined) this.openComment(anchor, null, event.clientX, event.clientY);
    });
  }

  private clearHover(): void {
    this.hoverPoint = undefined;
    this.hover.remove();
  }

  private updateHover(): void {
    const point = this.hoverPoint;
    if (point === undefined) return;
    const { page, x, y } = point;
    const block = page.blockAt(x, y);
    if (block === undefined) this.hover.remove();
    else page.mark(this.hover, block.anchor, page.element);
    const pending = page.prepareBlocks();
    if (pending === undefined || pending === this.hoverRequest) return;
    this.hoverRequest = pending;
    void pending.then(() => {
      if (this.hoverRequest !== pending) return;
      this.hoverRequest = undefined;
      this.updateHover();
    });
  }

  private selectBlock(page: PdfPage, x: number, y: number): void {
    const gesture = this.gesture;
    const select = (): void => {
      if (gesture !== this.gesture || !page.element.isConnected) return;
      const block = page.blockAt(x, y);
      if (block !== undefined) this.openComment(block.anchor, block.text, x, y);
    };
    const pending = page.prepareBlocks();
    if (pending === undefined) select();
    else void pending.then(select);
  }

  private pageFor(target: unknown): PdfPage | undefined {
    const element = target instanceof Element ? target : target instanceof Node ? target.parentElement : null;
    const page = element?.closest(".pdf-page");
    return page ? this.pages.get(page) : undefined;
  }

  private moveDrag(event: PointerEvent): void {
    const drag = this.drag;
    if (drag === null || drag.id !== event.pointerId) return;
    drag.moved ||= Math.hypot(drag.x - event.clientX, drag.y - event.clientY) >= DRAG_THRESHOLD;
    if (!drag.moved) return;
    const rect = drag.page.element.getBoundingClientRect();
    placeRectangle(drag.outline, rectangle(
      drag.x - rect.left, drag.y - rect.top,
      Math.max(0, Math.min(rect.width, event.clientX - rect.left)),
      Math.max(0, Math.min(rect.height, event.clientY - rect.top)),
    ));
  }

  private cancelDrag(): void {
    const drag = this.drag;
    if (drag === null) return;
    this.drag = null;
    drag.outline.remove();
    if (drag.page.element.hasPointerCapture(drag.id)) drag.page.element.releasePointerCapture(drag.id);
  }

  commentOnSelection(event?: MouseEvent): boolean {
    const selection = window.getSelection();
    if (selection === null || selection.isCollapsed || selection.rangeCount === 0) return false;
    const range = selection.getRangeAt(0).cloneRange();
    const page = this.pageFor(range.startContainer);
    if (page === undefined || (event !== undefined && this.pageFor(event.target) === undefined)) return false;
    const text = selectedTextFor(page.element);
    if (text === null) return false;
    const layer = page.element.querySelector(".textLayer");
    if (layer === null) return false;
    if (!layer.contains(range.endContainer)) range.setEnd(layer, layer.childNodes.length);
    const box = range.getBoundingClientRect();
    const anchor = page.anchor(box);
    if (anchor === undefined) return false;
    this.openComment(anchor, text, event?.clientX ?? box.left, event?.clientY ?? box.bottom);
    return true;
  }

  private openComment(anchor: PdfAnchor, selectedText: string | null, x: number, y: number): void {
    if (this.dialog === null || this.dialog.open) return;
    this.dialog.openPdf({
      element: null, page: location.href,
      selector: `#jamparty-pdf-anchor-${crypto.randomUUID()}`, selectedText, pdf: anchor,
    }, x, y);
  }
}
