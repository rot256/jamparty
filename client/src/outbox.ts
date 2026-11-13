// The comments this page sent or could not send: `queued` (with the
// share's id), `delivered`, or `unsent` (kept, sent again when the
// connection is back). Kept in sessionStorage across reloads.

import { MAX_OUTBOX, OUTBOX_KEY } from "./config";
import type { Outcome, Submission } from "./generated/api";

/** A comment as the page hands it to the joiner, without its ref. */
export type Draft = Omit<Submission, "ref" | "n">;

/** Hands a comment to the joiner; what became of it. */
export type Send = (submission: Omit<Submission, "n">) => Promise<Outcome>;

type OutboxState = "queued" | "delivered" | "unsent" | "sending";

export interface OutboxEntry {
  draft: Draft;
  /** The page's id for the comment; sending it again never queues it twice. */
  ref: string;
  id: string | null;
  state: OutboxState;
  /** When it was written (ms since the epoch). */
  at: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function parseDraft(value: unknown): Draft | null {
  if (!isRecord(value)) return null;
  const { page, selector, selectedText, comment } = value;
  if (typeof page !== "string" || typeof selector !== "string" || typeof comment !== "string") {
    return null;
  }
  return {
    page,
    selector,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment,
  };
}

function parseState(value: unknown): OutboxState {
  switch (value) {
    case "queued":
    case "delivered":
      return value;
    default:
      // A page that reloaded mid-send does not know whether it arrived;
      // sending again with the same ref is safe.
      return "unsent";
  }
}

function parseEntry(value: unknown): OutboxEntry | null {
  if (!isRecord(value)) return null;
  const draft = parseDraft(value.draft);
  if (draft === null) return null;
  return {
    draft,
    ref: typeof value.ref === "string" ? value.ref : crypto.randomUUID(),
    id: typeof value.id === "string" ? value.id : null,
    state: parseState(value.state),
    at: typeof value.at === "number" ? value.at : Date.now(),
  };
}

function load(): OutboxEntry[] {
  try {
    const stored: unknown = JSON.parse(sessionStorage.getItem(OUTBOX_KEY) ?? "[]");
    if (!Array.isArray(stored)) return [];
    return stored.map(parseEntry).filter((entry): entry is OutboxEntry => entry !== null);
  } catch {
    return [];
  }
}

export class Outbox {
  readonly entries: OutboxEntry[] = load();
  /** Called after every change, to redraw. */
  onChange: () => void = () => undefined;
  /**
   * The ids of this joiner's comments the share holds, as the joiner last
   * said; null while it does not know (a restarted joiner before the
   * share admitted it).
   */
  private undelivered: ReadonlySet<string> | null = null;

  constructor(private readonly send: Send) {}

  /** Persists the list (bounded) and redraws. */
  changed(): void {
    while (this.entries.length > MAX_OUTBOX) {
      const delivered = this.entries.findIndex((entry) => entry.state === "delivered");
      this.entries.splice(delivered === -1 ? 0 : delivered, 1);
    }
    try {
      sessionStorage.setItem(OUTBOX_KEY, JSON.stringify(this.entries));
    } catch {
      // The list stays in this document if storage is unavailable.
    }
    this.onChange();
  }

  /** A queued entry the share no longer holds was delivered. */
  private settle(entry: OutboxEntry): void {
    if (entry.state !== "queued" || entry.id === null || this.undelivered === null) return;
    if (!this.undelivered.has(entry.id)) entry.state = "delivered";
  }

  /**
   * The joiner's word on which of its comments the share holds (null: it
   * does not know yet, and nothing changes).
   */
  sync(undelivered: readonly string[] | null): void {
    this.undelivered = undelivered === null ? null : new Set(undelivered);
    for (const entry of this.entries) this.settle(entry);
    this.changed();
  }

  /**
   * Sends `draft` under `ref`, kept first: a reload before the joiner's
   * answer sends it again under the same ref, so it is queued once.
   * Resolves with the outcome once the entry reflects it; a refused
   * comment is taken out again (it goes back to the dialog).
   */
  async submit(draft: Draft, ref: string): Promise<Outcome> {
    const entry: OutboxEntry = { draft, ref, id: null, state: "sending", at: Date.now() };
    this.entries.push(entry);
    this.changed();
    const outcome = await this.send({ ref, ...draft });
    if (typeof outcome !== "string" && "refused" in outcome) {
      const index = this.entries.indexOf(entry);
      if (index !== -1) this.entries.splice(index, 1);
      this.changed();
    } else {
      this.take(entry, outcome);
    }
    return outcome;
  }

  /** Records what became of `entry`. */
  private take(entry: OutboxEntry, outcome: Outcome): void {
    if (typeof outcome !== "string" && "commented" in outcome) {
      entry.id = outcome.commented.id;
      entry.state = "queued";
      // Delivered already if the joiner's latest word does not hold it.
      this.settle(entry);
    } else {
      entry.state = "unsent";
    }
    this.changed();
  }

  async resend(entry: OutboxEntry): Promise<void> {
    if (entry.state !== "unsent") return;
    entry.state = "sending";
    this.changed();
    const outcome = await this.send({ ref: entry.ref, ...entry.draft });
    this.take(entry, outcome);
  }

  async resendAll(): Promise<void> {
    for (const entry of this.entries.filter((candidate) => candidate.state === "unsent")) {
      await this.resend(entry);
    }
  }
}
