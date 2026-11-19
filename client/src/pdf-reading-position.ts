import { nextTask } from "./pdf-blocks";
import type { PdfPage } from "./pdf-page";

const SEARCH_RADIUS = 4;
const MAX_MATCHES = 128;
const MIN_TEXT = 8;
const DISTINCTIVE_TEXT = 24;
const CONTEXT_LENGTH = 96;
const CONTEXT_MARGIN = 8;

interface ReadingAnchor {
  text: string;
  before: string;
  after: string;
  offset: number;
}

function normalized(text: string): string {
  return text.normalize("NFKC").replace(/[\s\u00ad\u2010\u2011-]/gu, "");
}

async function pageText(page: PdfPage, signal: AbortSignal) {
  const lines = await page.readingLines(signal);
  let text = "";
  const positions = lines.flatMap((line) => {
    const value = normalized(line.text);
    if (value.length === 0) return [];
    const start = text.length;
    text += value;
    return [{ ...line, start, end: text.length }];
  });
  return { text, positions };
}

async function capture(page: PdfPage, edge: number, signal: AbortSignal): Promise<ReadingAnchor | undefined> {
  const rect = page.element.getBoundingClientRect();
  const { text, positions } = await pageText(page, signal);
  const line = positions.find(({ bottom }) => rect.top + bottom * rect.height >= edge);
  if (line === undefined || line.end - line.start < MIN_TEXT) return;
  return {
    text: text.slice(line.start, line.end),
    before: text.slice(Math.max(0, line.start - CONTEXT_LENGTH), line.start),
    after: text.slice(line.end, line.end + CONTEXT_LENGTH),
    offset: rect.top + line.top * rect.height - edge,
  };
}

function contextScore(text: string, index: number, anchor: ReadingAnchor): number {
  let before = 0;
  let after = 0;
  while (before < anchor.before.length &&
    text[index - before - 1] === anchor.before[anchor.before.length - before - 1]) before++;
  const end = index + anchor.text.length;
  while (after < anchor.after.length && text[end + after] === anchor.after[after]) after++;
  return before + after;
}

export async function findReadingPosition(
  oldPage: PdfPage, edge: number, newPages: readonly PdfPage[], signal: AbortSignal,
): Promise<{ page: number; y: number; offset: number } | undefined> {
  const center = oldPage.number;
  if (center - SEARCH_RADIUS > newPages.length) return;
  const anchor = await capture(oldPage, edge, signal);
  if (anchor === undefined) return;

  const nearby = [center];
  for (let distance = 1; distance <= SEARCH_RADIUS; distance++) {
    nearby.push(center - distance, center + distance);
  }

  let best: { page: number; y: number; offset: number } | undefined;
  let bestScore = -Infinity;
  let runnerUp = -Infinity;
  let matches = 0;
  for (const number of nearby) {
    const page = newPages[number - 1];
    if (page === undefined) continue;
    await nextTask();
    signal.throwIfAborted();
    const { text, positions } = await pageText(page, signal);
    let line = 0;
    for (let index = text.indexOf(anchor.text); index !== -1; index = text.indexOf(anchor.text, index + 1)) {
      if (++matches > MAX_MATCHES) return;
      while ((positions[line]?.end ?? Infinity) <= index) line++;
      const position = positions[line];
      if (position === undefined) break;
      const score = contextScore(text, index, anchor);
      if (score > bestScore) {
        runnerUp = bestScore;
        bestScore = score;
        best = { page: number, y: position.top, offset: anchor.offset };
      } else {
        runnerUp = Math.max(runnerUp, score);
      }
    }
  }
  // Short lines need supporting text. Repeated lines need a clear context
  // advantage, regardless of the candidate's proximity to the old page.
  if (anchor.text.length + bestScore >= DISTINCTIVE_TEXT && bestScore - runnerUp >= CONTEXT_MARGIN) return best;
  return undefined;
}
