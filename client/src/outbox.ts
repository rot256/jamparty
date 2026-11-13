// The comments this page sent or could not send: `queued` (with the
// share's id), `delivered`, or `unsent` (kept, sent again when the
// connection is back). Kept in sessionStorage across reloads.

import { submit } from "./api";
import { MAX_OUTBOX, OUTBOX_KEY } from "./config";

/** Deliveries of not-yet-known comments the page remembers. */
const MAX_EARLY = 100;
import type { Queued, Submission } from "./generated/api";

/** A comment as the page hands it to the joiner, without its ref. */
export type Draft = Omit<Submission, "ref">;

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
  /** Called with the joiner's reply after a comment was queued. */
  onQueued: (queued: Queued) => void = () => undefined;

  /**
   * Ids delivered before this page heard they were queued (the delivery
   * can overtake the joiner's answer to the page); the latest few.
   */
  private early: string[] = [];

  /** A queued entry whose delivery came first is delivered. */
  private settle(entry: OutboxEntry): void {
    if (entry.state !== "queued" || entry.id === null) return;
    const index = this.early.indexOf(entry.id);
    if (index === -1) return;
    this.early.splice(index, 1);
    entry.state = "delivered";
  }

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

  /**
   * A comment about to be sent, kept first: a reload before the joiner's
   * answer sends it again under the same ref, so it is queued once.
   */
  sending(draft: Draft, ref: string): OutboxEntry {
    const entry: OutboxEntry = { draft, ref, id: null, state: "sending", at: Date.now() };
    this.entries.push(entry);
    this.changed();
    return entry;
  }

  /** The joiner queued `entry`. */
  accepted(entry: OutboxEntry, queued: Queued): void {
    entry.id = queued.id;
    entry.state = "queued";
    this.settle(entry);
    this.changed();
  }

  /** `entry` did not reach the joiner: kept, sent again later. */
  unsent(entry: OutboxEntry): void {
    entry.state = "unsent";
    this.changed();
  }

  /** `entry` was refused (rate limit, full queue): back to the dialog. */
  withdraw(entry: OutboxEntry): void {
    const index = this.entries.indexOf(entry);
    if (index !== -1) this.entries.splice(index, 1);
    this.changed();
  }

  async resend(entry: OutboxEntry): Promise<void> {
    if (entry.state !== "unsent") return;
    entry.state = "sending";
    this.changed();
    try {
      const queued = await submit({ ref: entry.ref, ...entry.draft });
      entry.id = queued.id;
      entry.state = "queued";
      this.settle(entry);
      this.onQueued(queued);
    } catch {
      entry.state = "unsent";
    }
    this.changed();
  }

  async resendAll(): Promise<void> {
    for (const entry of this.entries.filter((candidate) => candidate.state === "unsent")) {
      await this.resend(entry);
    }
  }

  markDelivered(ids: readonly string[]): void {
    const delivered = new Set(ids);
    for (const entry of this.entries) {
      if (entry.id !== null && delivered.delete(entry.id) && entry.state === "queued") {
        entry.state = "delivered";
      }
    }
    // Not known here yet: perhaps queued by a request still on its way.
    this.early.push(...delivered);
    this.early.splice(0, Math.max(0, this.early.length - MAX_EARLY));
    this.changed();
  }

  /** Every queued comment not in `undelivered` was delivered meanwhile. */
  markDeliveredExcept(undelivered: readonly string[]): void {
    const held = new Set(undelivered);
    this.markDelivered(
      this.entries
        .filter((entry) => entry.state === "queued" && entry.id !== null && !held.has(entry.id))
        .map((entry) => entry.id ?? ""),
    );
  }
}
