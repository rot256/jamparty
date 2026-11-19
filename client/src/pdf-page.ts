import { AnnotationMode, TextLayer } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PDFPageProxy, PageViewport, RenderTask } from "pdfjs-dist";
import type { PDFOperatorList } from "pdfjs-dist/types/src/display/api";

import { parsePdfAnchor, type PdfAnchor } from "./pdf-anchor";
import { detectPdfBlocks, readRuns, type PdfBlock } from "./pdf-blocks";
import { linesFor, type Bounds, type Run } from "./pdf-layout";

interface Rectangle {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export function placeRectangle(element: HTMLElement, rect: Rectangle): void {
  element.style.left = `${rect.left}px`;
  element.style.top = `${rect.top}px`;
  element.style.width = `${rect.right - rect.left}px`;
  element.style.height = `${rect.bottom - rect.top}px`;
}

function rectangle(x0: number, y0: number, x1: number, y1: number): Rectangle {
  return {
    left: Math.min(x0, x1), top: Math.min(y0, y1),
    right: Math.max(x0, x1), bottom: Math.max(y0, y1),
  };
}

const MAX_CANVAS_PIXELS = 8_000_000;

function roundPoint(value: number): number {
  return Math.round(Math.max(0, value) * 100) / 100;
}

export class PdfPage {
  readonly element = document.createElement("section");
  readonly marks = document.createElement("div");
  private readonly content = document.createElement("div");
  private mediaBox: readonly [number, number, number, number] | null = null;
  private natural: PageViewport;
  private page: PDFPageProxy | null = null;
  private preparing = false;
  private visible = false;
  private viewport: PageViewport;
  private renderTask: RenderTask | null = null;
  private textLayer: TextLayer | null = null;
  private operators: PDFOperatorList | undefined;
  private runs: Run[] | undefined;
  private blocks: PdfBlock[] | undefined;
  private blockRequest: { controller: AbortController; promise: Promise<void> } | undefined;

  constructor(
    readonly number: number,
    first: PDFPageProxy,
    private readonly getPage: () => Promise<PDFPageProxy>,
    private readonly pageReady: () => void,
    private readonly rendered: () => void,
  ) {
    this.natural = first.getViewport({ scale: 1 });
    this.viewport = this.natural;
    if (number === 1) this.setPage(first);
    this.element.id = `jamparty-pdf-page-${number}`;
    this.element.className = "pdf-page";
    this.element.setAttribute("aria-label", `Page ${number}`);
    this.marks.className = "pdf-marks";
    this.element.append(this.content, this.marks);
  }

  private setPage(page: PDFPageProxy): void {
    this.page = page;
    this.natural = page.getViewport({ scale: 1 });
    // The local PDF.js build exposes the worker's normalized MediaBox.
    this.mediaBox = (page as PDFPageProxy & {
      mediaBox: [number, number, number, number];
    }).mediaBox;
  }

  setVisible(visible: boolean): void {
    if (this.visible === visible) return;
    this.visible = visible;
    if (visible) this.render();
    else {
      this.blockRequest?.controller.abort();
      this.blockRequest = undefined;
      this.runs = undefined;
      this.blocks = undefined;
      this.clear();
      // Cancel drawing first. PDF.js defers cleanup if the operator stream is still active.
      this.page?.cleanup();
    }
  }

  resize(zoom: number | "fit", available: number): void {
    const scale = zoom === "fit" ? available / this.natural.width : zoom * 96 / 72;
    this.viewport = this.natural.clone({ scale });
    this.element.style.width = `${this.viewport.width}px`;
    this.element.style.height = `${this.viewport.height}px`;
    this.element.style.setProperty("--total-scale-factor", String(scale * this.natural.userUnit));
    this.clear();
  }

  private clear(): void {
    this.renderTask?.cancel();
    this.textLayer?.cancel();
    this.renderTask = null;
    this.textLayer = null;
    this.operators = undefined;
    this.content.replaceChildren();
    delete this.element.dataset.rendered;
  }

  render(): void {
    if (!this.visible || this.renderTask !== null || this.preparing) return;
    const page = this.page;
    if (page === null) {
      this.preparing = true;
      void this.getPage().then((loaded) => {
        this.setPage(loaded);
        this.preparing = false;
        this.pageReady();
      }).catch(() => {
        this.preparing = false;
        if (this.visible) this.element.dataset.rendered = "error";
      });
      return;
    }
    const viewport = this.viewport;
    const canvas = document.createElement("canvas");
    canvas.setAttribute("aria-hidden", "true");
    const ratio = Math.min(devicePixelRatio, 4096 / viewport.width, 4096 / viewport.height,
      Math.sqrt(MAX_CANVAS_PIXELS / (viewport.width * viewport.height)));
    canvas.width = Math.max(1, Math.floor(viewport.width * ratio));
    canvas.height = Math.max(1, Math.floor(viewport.height * ratio));
    const text = document.createElement("div");
    text.className = "textLayer";
    const layer = new TextLayer({
      textContentSource: page.streamTextContent(),
      container: text,
      viewport,
    });
    this.content.append(canvas, text);
    const task = page.render({
      canvas, viewport,
      transform: [canvas.width / viewport.width, 0, 0, canvas.height / viewport.height, 0, 0],
      annotationMode: AnnotationMode.DISABLE,
      operationsFilter: (_index, operators) => {
        // Reuse the display list instead of asking the worker to generate it again.
        this.operators ??= operators;
        return true;
      },
    });
    this.renderTask = task;
    this.textLayer = layer;
    void Promise.all([task.promise, layer.render()]).then(() => {
      if (this.renderTask === task) {
        this.element.dataset.rendered = "true";
        this.rendered();
      }
    }).catch(() => {
      if (this.renderTask === task) this.element.dataset.rendered = "error";
    });
  }

  private async textRuns(page: PDFPageProxy, signal: AbortSignal): Promise<Run[]> {
    signal.throwIfAborted();
    if (this.runs !== undefined) return this.runs;
    const runs = await readRuns(page, signal);
    signal.throwIfAborted();
    if (this.visible) this.runs = runs;
    return runs;
  }

  async readingLines(signal: AbortSignal): Promise<{ text: string; top: number; bottom: number }[]> {
    signal.throwIfAborted();
    if (this.page === null && this.element.isConnected) return [];
    // Search pages are detached. Load their actual dimensions before insertion.
    const page = this.page ?? await this.getPage();
    if (this.page === null) this.setPage(page);
    const runs = await this.textRuns(page, signal);
    const lines = linesFor(runs.filter((run) => run.horizontal));
    lines.push(...runs.filter((run) => !run.horizontal));
    return lines.map(({ bounds: [left, bottom, right, top], text }) => {
      const [, y1] = this.natural.convertToViewportPoint(left, bottom) as [number, number];
      const [, y2] = this.natural.convertToViewportPoint(right, top) as [number, number];
      return {
        text, top: Math.min(y1, y2) / this.natural.height,
        bottom: Math.max(y1, y2) / this.natural.height,
      };
    }).filter(({ top, bottom }) => top >= 0 && bottom <= 1)
      .sort((a, b) => a.top - b.top);
  }

  prepareBlocks(): Promise<void> | undefined {
    const page = this.page;
    if (this.blocks !== undefined || !this.visible || this.element.dataset.rendered !== "true" || page === null) return;
    if (this.blockRequest !== undefined) return this.blockRequest.promise;
    const controller = new AbortController();
    const runs = this.textRuns(page, controller.signal);
    const promise = detectPdfBlocks(page, this.operators, controller.signal, runs).then((blocks) => {
      if (!controller.signal.aborted) this.blocks = blocks;
    }).catch(() => {
      if (!controller.signal.aborted) this.blocks = [];
    }).finally(() => {
      if (this.blockRequest?.controller === controller) this.blockRequest = undefined;
    });
    this.blockRequest = { controller, promise };
    return promise;
  }

  blockAt(x: number, y: number): { anchor: PdfAnchor; text: string | null } | undefined {
    if (this.blocks === undefined || this.element.dataset.rendered !== "true") return;
    const rect = this.element.getBoundingClientRect();
    if (x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return;
    const [px, py] = this.viewport.convertToPdfPoint(x - rect.left, y - rect.top) as [number, number];
    let best: PdfBlock | undefined;
    let area = Infinity;
    for (const block of this.blocks) {
      const [left, bottom, right, top] = block.bounds;
      if (px < left || px > right || py < bottom || py > top) continue;
      const size = (right - left) * (top - bottom);
      if (size >= area) continue;
      area = size;
      best = block;
    }
    if (best === undefined) return;
    const anchor = this.pdfAnchor(best.bounds);
    return anchor === undefined ? undefined : { anchor, text: best.text };
  }

  anchor(rect: Rectangle): PdfAnchor | undefined {
    const page = this.element.getBoundingClientRect();
    const [ax, ay] = this.viewport.convertToPdfPoint(rect.left - page.left, rect.top - page.top) as [number, number];
    const [bx, by] = this.viewport.convertToPdfPoint(rect.right - page.left, rect.bottom - page.top) as [number, number];
    return this.pdfAnchor([Math.min(ax, bx), Math.min(ay, by), Math.max(ax, bx), Math.max(ay, by)]);
  }

  /** Clip to the visible page, then measure from the unrotated MediaBox. */
  private pdfAnchor([left, bottom, right, top]: Bounds): PdfAnchor | undefined {
    const media = this.mediaBox;
    if (media === null || this.page === null) return undefined;
    const [cropLeft, cropBottom, cropRight, cropTop] = this.page.view as Bounds;
    const { userUnit } = this.viewport;
    return parsePdfAnchor({
      page: this.number,
      box: {
        x0: roundPoint((Math.max(left, cropLeft) - media[0]) * userUnit),
        y0: roundPoint((media[3] - Math.min(top, cropTop)) * userUnit),
        x1: roundPoint((Math.min(right, cropRight) - media[0]) * userUnit),
        y1: roundPoint((media[3] - Math.max(bottom, cropBottom)) * userUnit),
      },
    });
  }

  mark(element: HTMLElement, anchor: PdfAnchor, layer: HTMLElement = this.marks): void {
    const media = this.mediaBox;
    if (media === null) return;
    const [mediaLeft, , , mediaTop] = media;
    const { userUnit } = this.viewport;
    const { x0, y0, x1, y1 } = anchor.box;
    const position = (x: number, y: number): [number, number] => {
      return this.viewport.convertToViewportPoint(
        mediaLeft + x / userUnit, mediaTop - y / userUnit,
      ) as [number, number];
    };
    const [left, top] = position(x0, y0);
    const [right, bottom] = position(x1, y1);
    placeRectangle(element, rectangle(left, top, right, bottom));
    layer.append(element);
  }
}

export { rectangle };
