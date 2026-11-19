import { OPS } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PDFOperatorList, PDFPageProxy, TextContent, TextItem, TextStyle } from "pdfjs-dist/types/src/display/api";

import { commentText } from "./capture";
import { linesFor, paragraphsFor, type Bounds, type Run } from "./pdf-layout";

type Matrix = [number, number, number, number, number, number];

export interface PdfBlock {
  bounds: Bounds;
  text: string | null;
}

const MAX_TEXT_ITEMS = 4096;
const MAX_TEXT_LENGTH = 65536;
const MAX_OPERATORS = 24000;
const MAX_IMAGES = 512;
const MAX_DEPTH = 64;

export function nextTask(): Promise<void> {
  return new Promise((resolve) => { setTimeout(resolve, 0); });
}

function transformBox(matrix: Matrix, box: Bounds): Bounds {
  const [a, b, c, d, e, f] = matrix;
  const [l, bottom, r, top] = box;
  const xs = [a*l + c*bottom + e, a*l + c*top + e, a*r + c*bottom + e, a*r + c*top + e];
  const ys = [b*l + d*bottom + f, b*l + d*top + f, b*r + d*bottom + f, b*r + d*top + f];
  return [Math.min(...xs), Math.min(...ys), Math.max(...xs), Math.max(...ys)];
}

function matrix(value: unknown): Matrix | undefined {
  if (!Array.isArray(value) && !(value instanceof Float32Array) && !(value instanceof Float64Array)) return;
  if (value.length === 6 && Array.from(value as ArrayLike<unknown>).every(Number.isFinite)) return value as Matrix;
  return undefined;
}

function multiply([a, b, c, d, e, f]: Matrix, [g, h, i, j, k, l]: Matrix): Matrix {
  return [a*g + c*h, b*g + d*h, a*i + c*j, b*i + d*j, a*k + c*l + e, b*k + d*l + f];
}

function textRun(item: TextItem, style: TextStyle | undefined): Run | undefined {
  const m = matrix(item.transform);
  if (m === undefined || !item.str.trim()) return;
  const [a, b, c, d, x, y] = m;
  const size = Math.hypot(c, d);
  const length = Math.hypot(a, b);
  if (!(size > 0 && length > 0)) return;
  const vertical = style?.vertical === true;
  const width = vertical ? item.height : item.width;
  if (!Number.isFinite(width) || width <= 0) return;
  const ascent = style && Number.isFinite(style.ascent) ? style.ascent : 0.8;
  const descent = style && Number.isFinite(style.descent) ? style.descent : -0.2;
  const direction: Matrix = vertical
    ? [b / length, -a / length, d, -c, x, y]
    : [a / length, b / length, c, d, x, y];
  const bounds = transformBox(direction, [0, Math.min(descent, -0.1), width, Math.max(ascent, 0.5)]);
  if (!bounds.every(Number.isFinite)) return;
  return {
    bounds, text: item.str, baseline: y, size,
    horizontal: !vertical && a > 0 && d > 0 && Math.abs(b) < 0.05 * length && Math.abs(c) < 0.05 * size,
    rtl: item.dir === "rtl",
  };
}

export async function readRuns(page: PDFPageProxy, signal: AbortSignal): Promise<Run[]> {
  const stream = page.streamTextContent() as ReadableStream<TextContent>;
  const reader = stream.getReader();
  const cancel = (): void => { void reader.cancel().catch(() => {}); };
  signal.addEventListener("abort", cancel, { once: true });
  const styles: TextContent["styles"] = Object.create(null) as TextContent["styles"];
  const runs: Run[] = [];
  let items = 0;
  let length = 0;
  try {
    for (;;) {
      signal.throwIfAborted();
      const { value, done } = await reader.read();
      if (done) return runs;
      Object.assign(styles, value.styles);
      for (const item of value.items) {
        if (++items > MAX_TEXT_ITEMS) return [];
        if (!("str" in item)) continue;
        length += item.str.length;
        if (length > MAX_TEXT_LENGTH) return [];
        const run = textRun(item, styles[item.fontName]);
        if (run !== undefined) runs.push(run);
      }
      await nextTask();
    }
  } finally {
    signal.removeEventListener("abort", cancel);
    cancel();
    reader.releaseLock();
  }
}

async function imageBounds(ops: PDFOperatorList, signal: AbortSignal): Promise<Bounds[]> {
  const { fnArray, argsArray } = ops;
  if (fnArray.length > MAX_OPERATORS) return [];
  const images: Bounds[] = [];
  const stack: Matrix[] = [];
  let ctm: Matrix = [1, 0, 0, 1, 0, 0];
  const add = (position = ctm): void => {
    const bounds = transformBox(position, [0, 0, 1, 1]);
    if (bounds.every(Number.isFinite)) images.push(bounds);
  };
  for (let index = 0; index < fnArray.length; index++) {
    if (index > 0 && index % 2048 === 0) {
      await nextTask();
      signal.throwIfAborted();
    }
    if (images.length >= MAX_IMAGES || stack.length >= MAX_DEPTH) return [];
    const args = argsArray[index] as unknown[] | null;
    switch (fnArray[index]) {
      case OPS.save:
      case OPS.beginGroup:
        stack.push(ctm);
        break;
      case OPS.paintFormXObjectBegin: {
        stack.push(ctm);
        const form = matrix(args?.[0]);
        if (form !== undefined) ctm = multiply(ctm, form);
        break;
      }
      case OPS.restore:
      case OPS.endGroup:
      case OPS.paintFormXObjectEnd:
        ctm = stack.pop() ?? ctm;
        break;
      case OPS.transform: {
        const change = matrix(args);
        if (change !== undefined) ctm = multiply(ctm, change);
        break;
      }
      case OPS.paintImageXObject:
      case OPS.paintInlineImageXObject:
      case OPS.paintImageMaskXObject:
        add();
        break;
      case OPS.paintImageMaskXObjectGroup:
      case OPS.paintInlineImageXObjectGroup: {
        const entries = args?.[fnArray[index] === OPS.paintImageMaskXObjectGroup ? 0 : 1] as
          { transform: unknown }[] | undefined;
        if (entries === undefined || entries.length > MAX_IMAGES - images.length) return [];
        for (const entry of entries) {
          const position = matrix(entry.transform);
          if (position !== undefined) add(multiply(ctm, position));
        }
        break;
      }
      case OPS.paintImageXObjectRepeat:
      case OPS.paintImageMaskXObjectRepeat: {
        const mask = fnArray[index] === OPS.paintImageMaskXObjectRepeat;
        const positions = args?.[mask ? 5 : 3] as Float32Array | undefined;
        if (positions === undefined || positions.length > 2 * (MAX_IMAGES - images.length)) return [];
        for (let i = 0; i + 1 < positions.length; i += 2) {
          const position = matrix([args?.[1], mask ? args?.[2] : 0, mask ? args?.[3] : 0,
            args?.[mask ? 4 : 2], positions[i], positions[i + 1]]);
          if (position !== undefined) add(multiply(ctm, position));
        }
        break;
      }
      case undefined:
      default:
        break;
    }
  }
  return images;
}

export async function detectPdfBlocks(
  page: PDFPageProxy, ops: PDFOperatorList | undefined, signal: AbortSignal,
  runsRequest: Promise<Run[]>,
): Promise<PdfBlock[]> {
  const runs = await runsRequest.catch(() => []);
  signal.throwIfAborted();
  const images = ops === undefined ? [] : await imageBounds(ops, signal);
  const lines = linesFor(runs.filter((run) => run.horizontal));
  await nextTask();
  signal.throwIfAborted();
  return [
    ...paragraphsFor(lines, page.view).map(({ bounds, text }) => ({ bounds, text: commentText(text) })),
    ...runs.filter((run) => !run.horizontal).map((run) => ({ bounds: run.bounds, text: commentText(run.text) })),
    ...images.map((bounds) => ({ bounds, text: null })),
  ];
}
