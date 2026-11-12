// The connection state the page shows, from the joiner's event stream:
// the stream itself (with a watchdog on its pings), the joiner's link to
// the share and the share's status line.

import { type ServerEvent, fetchStatus, openEvents, parseEvent } from "./api";
import { WATCHDOG_MS } from "./config";
import type { Status } from "./generated/api";

export type Colour = "green" | "amber" | "grey";

export interface ConnectionView {
  colour: Colour;
  /** What the state means, for people. */
  text: string;
  /** Set when the joiner cannot serve this page; nothing is connected then. */
  incompatible: string | null;
}

export interface Handlers {
  /** The state changed. */
  state(view: ConnectionView, recovered: boolean): void;
  /** `api/status`, read on every (re)connected stream. */
  status(status: Status): void;
  /** One event from the stream. */
  event(event: ServerEvent): void;
}

export class Connection {
  private events: EventSource | null = null;
  private lastEvent = Date.now();
  private streamOpen = false;
  private shareConnected = false;
  private statusLine: string | null = null;
  private incompatible: string | null = null;
  private colour: Colour = "amber";
  private watchdog: number | undefined;

  constructor(
    private readonly handlers: Handlers,
    /** This page as one of the joiner's tabs, when it can say its URL. */
    private readonly tab: () => { tab: string; url: string } | null,
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

  /** Reads `api/status` again (on every reconnection and after deliveries). */
  async refresh(): Promise<void> {
    const checked = await fetchStatus();
    if (checked === null) return;
    if ("incompatible" in checked) {
      this.refuse(checked.incompatible);
      return;
    }
    const status = checked.status;
    this.shareConnected = status.connected;
    this.statusLine = status.status;
    this.handlers.status(status);
    this.render();
  }

  private connect(): void {
    if (this.incompatible !== null) return;
    const events = openEvents(this.tab());
    this.events = events;
    events.addEventListener("open", () => {
      this.streamOpen = true;
      this.lastEvent = Date.now();
      this.render();
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
