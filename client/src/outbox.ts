// The page's comments and their progress, kept in sessionStorage.
// Each entry keeps the joiner key it was made under. A joiner that
// starts over has a new key, and the share tells comments apart by key
// and ref: one sent again under the new key would be queued twice. So
// entries of another key are never sent again by themselves (the person
// may send one again by hand), and their delivery is not followed: the
// share speaks of this key's comments only.

import { parseAttachments, upload, UploadError } from "./attachments";
import { JOINER_KEY, MAX_OUTBOX, OUTBOX_KEY } from "./config";
import type { Attachment, CommentOutcome, Submission } from "./generated/api";
import { parsePdfAnchor } from "./pdf-anchor";
import { isRecord } from "./json";
import { forgetFiles, keepFiles, pruneFiles, readFiles } from "./outbox-files";

/** A comment as the page hands it to the joiner, without its ref. */
export type Draft = Omit<Submission, "ref" | "n">;

/** Hands a comment to the joiner; what became of it. */
export type Send = (submission: Omit<Submission, "n">) => Promise<CommentOutcome>;

type OutboxState = "queued" | "delivered" | "unsent" | "sending" | "failed";

/** How an entry is shown: its state, or `earlier` for one queued under another joiner key. */
export type ShownState = OutboxState | "earlier";

export interface OutboxEntry {
  draft: Draft;
  /** The page's id for the comment; sending it again never queues it twice. */
  ref: string;
  error?: string;
  /** Names of the files held in IndexedDB under this ref. */
  files?: string[];
  uploaded?: boolean;
  uploadProgress?: Attachment[];
  id: string | null;
  state: OutboxState;
  /** The joiner key it was made under; null if not known. */
  key: string | null;
  /** When it was written (ms since the epoch). */
  at: number;
}

function parseDraft(value: unknown): Draft | null {
  if (!isRecord(value)) return null;
  const { page, selector, selectedText, comment, choice } = value;
  if (typeof page !== "string" || typeof selector !== "string" || typeof comment !== "string") {
    return null;
  }
  const attachments = value.attachments === undefined ? [] : parseAttachments(value.attachments);
  if (attachments === null) return null;
  const pdf = parsePdfAnchor(value.pdf);
  return {
    page,
    ...(pdf === undefined ? {} : { pdf }),
    selector,
    selectedText: typeof selectedText === "string" ? selectedText : null,
    comment,
    ...(attachments.length > 0 ? { attachments } : {}),
    ...(typeof choice === "string" ? { choice } : {}),
  };
}

function parseState(value: unknown): OutboxState {
  switch (value) {
    case "queued":
    case "delivered":
    case "failed":
    case "sending":
      return value;
    default:
      return "unsent";
  }
}

function parseEntry(value: unknown): OutboxEntry | null {
  if (!isRecord(value)) return null;
  const draft = parseDraft(value.draft);
  if (draft === null) return null;
  const progress = parseAttachments(value.uploadProgress);
  return {
    draft,
    ref: typeof value.ref === "string" ? value.ref : crypto.randomUUID(),
    id: typeof value.id === "string" ? value.id : null,
    state: parseState(value.state),
    key: typeof value.key === "string" ? value.key : null,
    ...(typeof value.error === "string" ? { error: value.error } : {}),
    ...(Array.isArray(value.files) && value.files.every((name) => typeof name === "string")
      ? { files: value.files } : {}),
    ...(progress === null ? {} : { uploadProgress: progress }),
    ...(value.uploaded === true ? { uploaded: true } : {}),
    at: typeof value.at === "number" ? value.at : Date.now(),
  };
}

/** The key of the joiner this tab last heard from, across reloads. */
function lastKey(): string | null {
  try {
    return sessionStorage.getItem(JOINER_KEY);
  } catch {
    return null;
  }
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
  onInitialOutcome: (outcome: CommentOutcome) => void = () => undefined;
  private readonly active = new Set<OutboxEntry>();
  /** Serializes file transfers within this tab. The joiner also caps concurrent uploads. */
  private uploads: Promise<void> = Promise.resolve();
  /**
   * The ids of this joiner's comments the share holds, as the joiner last
   * said; null while it does not know (a restarted joiner before the
   * share admitted it).
   */
  private undelivered: ReadonlySet<string> | null = null;
  /** This joiner's key, once a hello said it. */
  private key: string | null = null;
  /**
   * The key a comment is made under: this joiner's, or before its hello
   * (a comment written as the page loads), that of the joiner this tab
   * last heard from, which it most likely is.
   */
  private stamp: string | null = lastKey();

  private retryTimer: number | undefined;

  constructor(private readonly send: Send, private readonly reachable: () => boolean) {
    pruneFiles();
  }

  private retryUploads(): void {
    this.retryTimer ??= window.setTimeout(() => {
      this.retryTimer = undefined;
      if (this.reachable()) void this.resendAll();
    }, 2000);
  }

  /** The joiner's key, from its hello or link (null: not known yet, and kept). */
  setKey(key: string | null): void {
    if (key === null || key === this.key) return;
    this.key = key;
    this.stamp = key;
    // Comments made with no key known never reached a joiner (the socket
    // sends nothing before its hello): they are this joiner's to send.
    for (const entry of this.entries) {
      if (entry.key === null) entry.key = key;
    }
    try {
      sessionStorage.setItem(JOINER_KEY, key);
    } catch {
      // Comments made before the next hello then count as another key's.
    }
    this.changed();
  }

  /** Whether `entry` was made under another joiner key than this one's. */
  private foreign(entry: OutboxEntry): boolean {
    return this.key !== null && entry.key !== this.key;
  }

  /** How `entry` is shown. */
  shown(entry: OutboxEntry): ShownState {
    return (entry.state === "queued" || entry.state === "sending") && this.foreign(entry)
      ? "earlier" : entry.state;
  }

  private persist(): void {
    sessionStorage.setItem(OUTBOX_KEY, JSON.stringify(this.entries));
  }

  /** Persists state changes and redraws. */
  changed(): void {
    try {
      this.persist();
    } catch {
      // The entry and its last saved ref remain usable in this document.
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

  /** Whether adding a new comment would displace one the browser must keep. */
  private full(replaces?: OutboxEntry): boolean {
    return this.entries.length >= MAX_OUTBOX &&
      (replaces === undefined || !this.entries.includes(replaces)) && this.droppable() === -1;
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

  private fullError(): string {
    if (this.entries.some((entry) => entry.state === "failed")) {
      return "The comment list is full. Edit or discard a refused comment, or wait for a comment to queue.";
    }
    return `${MAX_OUTBOX} comments are waiting to be sent; this one can go once they have`;
  }

  /** Make room and persist the new ref before starting any network work. */
  private add(draft: Draft, ref: string, files: File[] = [], replaces?: OutboxEntry): OutboxEntry {
    if (this.full(replaces)) throw new Error(this.fullError());
    const entry: OutboxEntry = {
      draft,
      ref,
      id: null,
      state: "sending",
      key: this.stamp,
      at: Date.now(),
      ...(files.length === 0 ? {} : { files: files.map((file) => file.name) }),
    };
    const replaced = replaces === undefined ? -1 : this.entries.indexOf(replaces);
    const index = replaced !== -1 ? replaced :
      this.entries.length >= MAX_OUTBOX ? this.droppable() : -1;
    const [removed] = index === -1 ? [] : this.entries.splice(index, 1);
    this.entries.push(entry);
    try {
      this.persist();
    } catch {
      this.entries.pop();
      if (removed !== undefined) this.entries.splice(index, 0, removed);
      throw new Error("Could not save the comment in this tab.");
    }
    if (removed?.files !== undefined) void forgetFiles(removed.ref);
    this.onChange();
    return entry;
  }

  /** Save a comment locally and start delivery without waiting for the share. */
  async enqueue(draft: Draft, files: File[], replaces?: OutboxEntry): Promise<void> {
    if (this.full(replaces)) throw new Error(this.fullError());
    if (replaces?.files !== undefined &&
      (await this.savedFiles(replaces)).accepted !== undefined) {
      throw new Error("This comment has already been queued.");
    }
    const ref = crypto.randomUUID();
    if (files.length > 0 && !await keepFiles(ref, files, replaces?.ref).catch(() => false)) {
      throw new Error("Could not save the files in this browser. Free space or remove an attachment.");
    }
    let entry: OutboxEntry;
    try {
      entry = this.add(draft, ref, files, replaces);
    } catch (error) {
      if (files.length > 0) await forgetFiles(ref);
      throw error;
    }
    void this.deliver(entry).then((outcome) => {
      if (!this.foreign(entry)) this.onInitialOutcome(outcome);
    });
  }

  /** Choice buttons still wait for a reply before dismissing the question. */
  async submit(draft: Draft, ref: string): Promise<CommentOutcome> {
    let entry: OutboxEntry;
    try {
      entry = this.add(draft, ref);
    } catch (error) {
      return { refused: { error: error instanceof Error ? error.message : "Could not save the choice." } };
    }
    return this.deliver(entry);
  }

  discard(entry: OutboxEntry): void {
    const index = this.entries.indexOf(entry);
    if (index === -1) return;
    this.entries.splice(index, 1);
    if (entry.files !== undefined) void forgetFiles(entry.ref);
    this.changed();
  }

  private accept(entry: OutboxEntry, id: string): void {
    entry.id = id;
    entry.state = "queued";
    delete entry.error;
    delete entry.files;
    delete entry.uploaded;
    delete entry.uploadProgress;
    this.settle(entry);
  }

  async savedFiles(entry: OutboxEntry): ReturnType<typeof readFiles> {
    const saved = await readFiles(entry.ref);
    if (saved.accepted !== undefined) {
      entry.draft.attachments = saved.accepted.attachments;
      this.accept(entry, saved.accepted.id);
      this.changed();
    }
    return saved;
  }

  private async uploadFiles(entry: OutboxEntry, files: File[]): Promise<void> {
    if (files.length !== entry.files?.length) {
      throw new UploadError("The saved files are unavailable. Attach them again.", false);
    }
    const uploaded = entry.uploadProgress ?? [];
    entry.uploadProgress = uploaded;
    for (const file of files.slice(uploaded.length)) {
      try {
        uploaded.push(await upload(file));
        this.changed();
      } catch (error) {
        const reason = error instanceof Error ? error.message : "Could not upload the attachment.";
        throw new UploadError(`${file.name} — ${reason}`, error instanceof UploadError && error.transient);
      }
    }
    entry.draft.attachments = [...(entry.draft.attachments ?? []), ...uploaded];
    entry.uploaded = true;
    delete entry.uploadProgress;
    this.changed();
  }

  private async sendEntry(entry: OutboxEntry): Promise<CommentOutcome> {
    if (entry.files !== undefined) {
      const work = this.uploads.then(async () => {
        const { files, accepted } = await readFiles(entry.ref);
        if (accepted === undefined && entry.uploaded !== true && !this.foreign(entry)) {
          await this.uploadFiles(entry, files);
        }
        return accepted;
      });
      this.uploads = work.then(() => undefined, () => undefined);
      try {
        const accepted = await work;
        if (accepted !== undefined) {
          entry.draft.attachments = accepted.attachments;
          return { commented: { id: accepted.id } };
        }
      } catch (error) {
        if (!(error instanceof UploadError) || error.transient) {
          this.retryUploads();
          return { unsent: { error: error instanceof Error ? error.message : "Could not read the files." } };
        }
        return { refused: { error: error.message } };
      }
    }
    if (this.foreign(entry)) return { unsent: { error: "The joiner restarted." } };
    return this.send({ ref: entry.ref, ...entry.draft });
  }

  private async deliver(entry: OutboxEntry): Promise<CommentOutcome> {
    this.active.add(entry);
    let outcome: CommentOutcome;
    try {
      outcome = await this.sendEntry(entry);
    } catch {
      outcome = { unsent: { error: "Could not send the comment." } };
    }
    if ("refused" in outcome) {
      if (entry.draft.choice !== undefined && outcome.refused.choiceRefused === true) {
        this.active.delete(entry);
        this.discard(entry);
        return outcome;
      }
      entry.state = entry.draft.choice === undefined ? "failed" : "unsent";
      entry.error = outcome.refused.error;
    } else if ("commented" in outcome) {
      if (entry.files !== undefined) {
        await forgetFiles(entry.ref, { id: outcome.commented.id, attachments: entry.draft.attachments ?? [] });
      }
      this.accept(entry, outcome.commented.id);
    } else if (!this.foreign(entry)) {
      entry.state = "unsent";
    }
    this.active.delete(entry);
    this.changed();
    return outcome;
  }

  canResend(entry: OutboxEntry): boolean {
    return !this.active.has(entry) && (entry.state === "unsent" || entry.state === "sending");
  }

  /** Send again under the same ref. Only a manual retry can adopt an earlier key. */
  async resend(entry: OutboxEntry): Promise<void> {
    if (!this.canResend(entry)) return;
    entry.key = this.stamp;
    entry.state = "sending";
    const work = this.deliver(entry);
    this.changed();
    await work;
  }

  /** Sends every unsent or interrupted entry made under this joiner key. */
  async resendAll(): Promise<void> {
    const unsent = this.entries.filter(
      (entry) => this.canResend(entry) && !this.foreign(entry),
    );
    for (const entry of unsent) {
      if (!this.foreign(entry)) await this.resend(entry);
    }
  }
}
