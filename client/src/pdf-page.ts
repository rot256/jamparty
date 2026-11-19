import { AnnotationMode, TextLayer } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PDFPageProxy, PageViewport, RenderTask } from "pdfjs-dist";

import { parsePdfAnchor, type PdfAnchor } from "./pdf-anchor";

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

  constructor(
    readonly number: number,
    first: PDFPageProxy,
    private readonly getPage: () => Promise<PDFPageProxy>,
    private readonly pageReady: () => void,
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
    });
    this.renderTask = task;
    this.textLayer = layer;
    void Promise.all([task.promise, layer.render()]).then(() => {
      if (this.renderTask === task) this.element.dataset.rendered = "true";
    }).catch(() => {
      if (this.renderTask === task) this.element.dataset.rendered = "error";
    });
  }

  /** Clip to the visible page, then measure from the unrotated MediaBox. */
  anchor(rect: Rectangle): PdfAnchor | undefined {
    const media = this.mediaBox;
    if (media === null) return undefined;
    const [mediaLeft, , , mediaTop] = media;
    const { userUnit } = this.viewport;
    const page = this.element.getBoundingClientRect();
    const point = (x: number, y: number): { x: number; y: number } => {
      const [pdfX, pdfY] = this.viewport.convertToPdfPoint(
        Math.max(0, Math.min(page.width, x - page.left)),
        Math.max(0, Math.min(page.height, y - page.top)),
      ) as [number, number];
      return { x: roundPoint((pdfX - mediaLeft) * userUnit), y: roundPoint((mediaTop - pdfY) * userUnit) };
    };
    const a = point(rect.left, rect.top);
    const b = point(rect.right, rect.bottom);
    const box = rectangle(a.x, a.y, b.x, b.y);
    return parsePdfAnchor({
      page: this.number, box: { x0: box.left, y0: box.top, x1: box.right, y1: box.bottom },
    });
  }

  mark(element: HTMLElement, anchor: PdfAnchor): void {
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
    this.marks.append(element);
  }
}

export { rectangle };
