// The comments this page sent or could not send: `queued` (with the
// share's id), `delivered`, or `unsent` (kept, sent again when the
// connection is back). Kept in sessionStorage across reloads, at most
// MAX_OUTBOX: delivered ones go first, then queued ones (the share has
// those); unsent ones are never dropped, so a new comment is refused
// while the outbox is full of them.
//
// Each entry keeps the joiner key it was made under. A joiner that
// starts over has a new key, and the share tells comments apart by key
// and ref: one sent again under the new key would be queued twice. So
// entries of another key are never sent again by themselves (the person
// may send one again by hand), and their delivery is not followed: the
// share speaks of this key's comments only.

import { MAX_OUTBOX, OUTBOX_KEY } from "./config";
import type { CommentOutcome, Submission } from "./generated/api";
import { isRecord } from "./json";

/** A comment as the page hands it to the joiner, without its ref. */
export type Draft = Omit<Submission, "ref" | "n">;

/** Hands a comment to the joiner; what became of it. */
export type Send = (submission: Omit<Submission, "n">) => Promise<CommentOutcome>;

type OutboxState = "queued" | "delivered" | "unsent" | "sending";

/** How an entry is shown: its state, or `earlier` for one queued under another joiner key. */
export type ShownState = OutboxState | "earlier";

export interface OutboxEntry {
  draft: Draft;
  /** The page's id for the comment; sending it again never queues it twice. */
  ref: string;
  id: string | null;
  state: OutboxState;
  /** The joiner key it was made under; null if not known. */
  key: string | null;
  /** When it was written (ms since the epoch). */
  at: number;
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
    key: typeof value.key === "string" ? value.key : null,
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
  /** This joiner's key, once a hello said it. */
  private key: string | null = null;

  constructor(private readonly send: Send) {}

  /** The joiner's key, from its hello or link (null: not known yet, and kept). */
  setKey(key: string | null): void {
    if (key === null || key === this.key) return;
    this.key = key;
    this.changed();
  }

  /** Whether `entry` was made under another joiner key than this one's. */
  private foreign(entry: OutboxEntry): boolean {
    return this.key !== null && entry.key !== this.key;
  }

  /** How `entry` is shown. */
  shown(entry: OutboxEntry): ShownState {
    return entry.state === "queued" && this.foreign(entry) ? "earlier" : entry.state;
  }

  /** Persists the list (bounded) and redraws. */
  changed(): void {
    while (this.entries.length > MAX_OUTBOX) {
      const index = this.droppable();
      if (index === -1) break;
      this.entries.splice(index, 1);
    }
    try {
      sessionStorage.setItem(OUTBOX_KEY, JSON.stringify(this.entries));
    } catch {
      // The list stays in this document if storage is unavailable.
    }
    this.onChange();
  }

  /**
   * The oldest entry the page may forget: a delivered one, else a queued
   * one (the share holds it); -1 if every entry is still to be sent.
   */
  private droppable(): number {
    const delivered = this.entries.findIndex((entry) => entry.state === "delivered");
    if (delivered !== -1) return delivered;
    return this.entries.findIndex((entry) => entry.state === "queued");
  }

  /** Whether the outbox holds as many comments still to send as it keeps. */
  get full(): boolean {
    const unsent = this.entries.filter(
      (entry) => entry.state === "unsent" || entry.state === "sending",
    );
    return unsent.length >= MAX_OUTBOX;
  }

  /** A queued entry the share no longer holds was delivered. */
  private settle(entry: OutboxEntry): void {
    if (entry.state !== "queued" || entry.id === null || this.undelivered === null) return;
    // The share's word is about this key's comments only.
    if (this.foreign(entry)) return;
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
   * comment is taken out again (it goes back to the dialog). Refused
   * without sending while the outbox is full of comments still to send.
   */
  async submit(draft: Draft, ref: string): Promise<CommentOutcome> {
    if (this.full) {
      return {
        refused: {
          error: `${MAX_OUTBOX} comments are waiting to be sent; this one can go once they have`,
        },
      };
    }
    const entry: OutboxEntry = {
      draft,
      ref,
      id: null,
      state: "sending",
      key: this.key,
      at: Date.now(),
    };
    this.entries.push(entry);
    this.changed();
    const outcome = await this.send({ ref, ...draft });
    if ("refused" in outcome) {
      const index = this.entries.indexOf(entry);
      if (index !== -1) this.entries.splice(index, 1);
      this.changed();
    } else {
      this.take(entry, outcome);
    }
    return outcome;
  }

  /** Records what became of `entry`. */
  private take(entry: OutboxEntry, outcome: CommentOutcome): void {
    if ("commented" in outcome) {
      entry.id = outcome.commented.id;
      entry.state = "queued";
      // Delivered already if the joiner's latest word does not hold it.
      this.settle(entry);
    } else {
      entry.state = "unsent";
    }
    this.changed();
  }

  /**
   * Sends an unsent entry again. One of another joiner key goes under
   * this one's from now on: only ever by hand, since the share may have
   * queued it under the other.
   */
  async resend(entry: OutboxEntry): Promise<void> {
    if (entry.state !== "unsent") return;
    entry.key = this.key;
    entry.state = "sending";
    this.changed();
    const outcome = await this.send({ ref: entry.ref, ...entry.draft });
    this.take(entry, outcome);
  }

  /** Sends every unsent entry of this joiner key again. */
  async resendAll(): Promise<void> {
    const unsent = this.entries.filter(
      (candidate) => candidate.state === "unsent" && !this.foreign(candidate),
    );
    for (const entry of unsent) {
      await this.resend(entry);
    }
  }
}
