// The connection state the page shows, from the joiner's event stream:
// the stream itself (with a watchdog on its pings), the joiner's link to
// the share and the share's status line.

import { type ServerEvent, fetchStatus, openEvents, parseEvent } from "./api";
import { WATCHDOG_MS } from "./config";
import type { Stamped, Status } from "./generated/api";

export type Colour = "green" | "amber" | "grey";

export interface ConnectionView {
  colour: Colour;
  /** What the state means, for people. */
  text: string;
  /** Set when the joiner cannot serve this page; nothing is connected then. */
  incompatible: string | null;
}

/** The parts of the joiner's view a status snapshot is taken in. */
export type Part = "connection" | "presence" | "mine" | "settings" | "reload";

const PARTS: readonly Part[] = ["connection", "presence", "mine", "settings", "reload"];

function unapplied(): Record<Part, number> {
  return { connection: 0, presence: 0, mine: 0, settings: 0, reload: 0 };
}

/**
 * The part of the view an event changes; none for answers (merged, never
 * replaced) and cursors (not in a snapshot).
 */
function partOf(event: Stamped): Part | null {
  switch (event.type) {
    case "share":
    case "status":
      return "connection";
    case "presence":
      return "presence";
    case "delivered":
      return "mine";
    case "settings":
      return "settings";
    case "reload":
      return "reload";
    case "answer":
    case "cursor":
      return null;
  }
}

export interface Handlers {
  /** The state changed. */
  state(view: ConnectionView, recovered: boolean): void;
  /**
   * `api/status`, read on every (re)connected stream: the parts of it to
   * take (the others are older than what the page has; answers are always
   * merged).
   */
  status(status: Status, parts: ReadonlySet<Part>): void;
  /** One event from the stream. */
  event(event: ServerEvent): void;
  /** The stream (re)opened: this page is one of the joiner's tabs again. */
  opened(): void;
}

export class Connection {
  private events: EventSource | null = null;
  private lastEvent = Date.now();
  private streamOpen = false;
  private shareConnected = false;
  private statusLine: string | null = null;
  private incompatible: string | null = null;
  private colour: Colour = "amber";
  /**
   * The joiner process the event stream comes from (its epoch), and per
   * part of the joiner's view the newest revision in it this page has
   * applied (from events, comments it queued, settings it saved). An
   * `api/status` is applied part by part: each part unless the page has
   * already applied something newer for that same part, so an unrelated
   * event (someone's rename, a cursor) never costs a snapshot. Revisions
   * count per joiner process: one restarted on the same port starts
   * again, so a new epoch starts afresh, and answers from another epoch
   * are not applied.
   */
  private epoch: string | null = null;
  private applied: Record<Part, number> = unapplied();
  /** A status read is wanted again once the current one is in. */
  private rereadQueued = false;
  private reading = false;
  private watchdog: number | undefined;

  constructor(
    private readonly handlers: Handlers,
    /** This page as one of the joiner's tabs: its id and URL now. */
    private readonly tab: () => { tab: string; url: string | null; seq: number },
  ) {}

  /** Checks the joiner's page API, then connects the event stream. */
  async start(): Promise<void> {
    const checked = await fetchStatus();
    if (checked !== null && "incompatible" in checked) {
      this.refuse(checked.incompatible);
      return;
    }
    this.connect();
    // The joiner pings every 15 s; a silent stream is a dead one.
    this.watchdog = window.setInterval(() => {
      if (Date.now() - this.lastEvent < WATCHDOG_MS) return;
      this.events?.close();
      this.streamOpen = false;
      this.render();
      this.lastEvent = Date.now();
      this.connect();
    }, 5000);
  }

  get view(): ConnectionView {
    let colour: Colour = "green";
    let text = "connected";
    if (this.incompatible !== null) {
      colour = "grey";
      text = this.incompatible;
    } else if (!this.streamOpen) {
      colour = "amber";
      text = "reconnecting to jampot join…";
    } else if (!this.shareConnected) {
      colour = "grey";
      text = "share unreachable";
    } else if (this.statusLine !== null) {
      colour = "grey";
      text = this.statusLine;
    }
    return { colour, text, incompatible: this.incompatible };
  }

  /** Whether comments can go out now. */
  get connected(): boolean {
    return this.colour === "green";
  }

  /** Whether the page has its event stream (so it is one of the joiner's tabs). */
  get streaming(): boolean {
    return this.streamOpen;
  }

  private refuse(reason: string): void {
    this.incompatible = reason;
    this.events?.close();
    this.events = null;
    if (this.watchdog !== undefined) clearInterval(this.watchdog);
    this.render();
  }

  private render(): void {
    const view = this.view;
    const recovered = view.colour === "green" && this.colour !== "green";
    this.colour = view.colour;
    this.handlers.state(view, recovered);
  }

  /**
   * Whether an answer about `part`, stamped `epoch` and `rev`, is still
   * news (nothing newer applied for that part since); if so it counts as
   * applied.
   */
  fresh(epoch: string, rev: number, part: Part): boolean {
    if (epoch !== this.epoch || rev < this.applied[part]) return false;
    this.applied[part] = rev;
    return true;
  }

  /**
   * Reads `api/status` again (on every reconnection and after deliveries);
   * one read at a time, and one more queued behind it at most.
   */
  async refresh(): Promise<void> {
    if (this.reading) {
      this.rereadQueued = true;
      return;
    }
    this.reading = true;
    try {
      do {
        this.rereadQueued = false;
        await this.read();
      } while (this.wantsReread());
    } finally {
      this.reading = false;
    }
  }

  /** Whether another read was asked for while one was out. */
  private wantsReread(): boolean {
    return this.rereadQueued;
  }

  private async read(): Promise<void> {
    const checked = await fetchStatus();
    if (checked === null) return;
    if ("incompatible" in checked) {
      this.refuse(checked.incompatible);
      return;
    }
    const status = checked.status;
    if (this.epoch === null) this.epoch = status.epoch;
    // From another joiner process: its own event stream brings its state
    // (a new epoch reads the status again).
    if (status.epoch !== this.epoch) return;
    const parts = new Set<Part>();
    for (const part of PARTS) {
      if (this.fresh(status.epoch, status.rev, part)) parts.add(part);
    }
    if (parts.has("connection")) {
      this.shareConnected = status.connected;
      this.statusLine = status.status;
    }
    this.handlers.status(status, parts);
    this.render();
    // Delivery events say which comments, not how many are pending: when
    // a newer one stood against this read, read again (it is newer than
    // those, so this ends).
    if (!parts.has("mine")) this.rereadQueued = true;
  }

  private connect(): void {
    if (this.incompatible !== null) return;
    const { tab, url, seq } = this.tab();
    const events = openEvents(tab, url, seq);
    this.events = events;
    events.addEventListener("open", () => {
      this.streamOpen = true;
      this.lastEvent = Date.now();
      this.render();
      // A stream that reconnected by itself still names the page it
      // opened on: say where this page is now.
      this.handlers.opened();
      void this.refresh();
    });
    events.addEventListener("error", () => {
      this.streamOpen = false;
      this.render();
    });
    events.addEventListener("message", (message: MessageEvent<string>) => {
      this.lastEvent = Date.now();
      const event = parseEvent(message.data);
      if (event === null) return;
      if (event.type !== "ping") {
        if (event.epoch !== this.epoch) {
          // Another joiner process (restarted on this port): start afresh.
          const known = this.epoch !== null;
          this.epoch = event.epoch;
          this.applied = unapplied();
          if (known) void this.refresh();
        }
        const part = partOf(event);
        if (part !== null) this.applied[part] = Math.max(this.applied[part], event.rev);
      }
      switch (event.type) {
        case "share":
          this.shareConnected = event.connected;
          this.render();
          if (event.connected) void this.refresh();
          break;
        case "status":
          this.statusLine = event.line;
          this.render();
          break;
        case "delivered":
          void this.refresh();
          break;
        case "answer":
        case "reload":
        case "presence":
        case "settings":
        case "cursor":
        case "ping":
          break;
      }
      this.handlers.event(event);
    });
  }
}
