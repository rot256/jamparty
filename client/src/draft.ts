// The comment being written, kept in sessionStorage while the dialog is
// open, so a reload the page did not ask for (a dev server's live reload)
// does not lose it. Cleared when it is sent (the outbox has it then) or the
// dialog closes.

import { DRAFT_KEY } from "./config";
import { isRecord } from "./json";

/** What the dialog was opened on. */
export type DraftKind = "element" | "page" | "answer";

/** What the dialog was opened on, as it was opened. */
export interface DraftTarget {
  kind: DraftKind;
  /** On the page, `jamparty-agent:<id>` for an answer, `""` for the page. */
  selector: string;
}

export interface SavedDraft {
  /** The page it was written on (its full URL). */
  page: string;
  target: DraftTarget;
  /** The target was gone after a reload: it goes to the page, and stays so. */
  unavailable: boolean;
  selectedText: string | null;
  comment: string;
  /** File contents only live in the open dialog. */
  fileCount: number;
}

function parseKind(value: unknown): DraftKind | null {
  switch (value) {
    case "element":
    case "page":
    case "answer":
      return value;
    default:
      return null;
  }
}

function parseTarget(value: unknown): DraftTarget | null {
  if (!isRecord(value)) return null;
  const kind = parseKind(value.kind);
  const { selector } = value;
  if (kind === null || typeof selector !== "string") return null;
  return { kind, selector };
}

function parse(value: unknown): SavedDraft | null {
  if (!isRecord(value)) return null;
  const { page, unavailable, selectedText, comment } = value;
  const target = parseTarget(value.target);
  if (typeof page !== "string" || typeof comment !== "string" || target === null) return null;
  return {
    page,
    target,
    unavailable: unavailable === true,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment,
    fileCount: typeof value.fileCount === "number" && value.fileCount > 0 ? value.fileCount : 0,
  };
}

export function saveDraft(draft: SavedDraft): void {
  try {
    sessionStorage.setItem(DRAFT_KEY, JSON.stringify(draft));
  } catch {
    // No storage (private mode, quota): the draft lives only in the dialog.
  }
}

export function clearDraft(): void {
  try {
    sessionStorage.removeItem(DRAFT_KEY);
  } catch {
    // Nothing was kept.
  }
}

/** Whether `page` is this page: path, query and fragment alike. */
function isThisPage(page: string): boolean {
  try {
    const url = new URL(page);
    const here = window.location;
    return url.pathname === here.pathname && url.search === here.search && url.hash === here.hash;
  } catch {
    return false;
  }
}

/** The draft left on this page, if any. */
export function loadDraft(): SavedDraft | null {
  try {
    const draft = parse(JSON.parse(sessionStorage.getItem(DRAFT_KEY) ?? "null"));
    return draft !== null && isThisPage(draft.page) ? draft : null;
  } catch {
    return null;
  }
}
