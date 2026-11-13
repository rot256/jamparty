// The page's one socket to the joiner (`api/socket`). The joiner sends its
// whole state first (`hello`), then every change in order; the page says
// where it is and sends its pointer, comments and settings changes, each
// request answered by a `reply`. A socket that closes, or stays silent
// past the watchdog, is opened again and starts over from a new hello.

import { parseMessage } from "./api";
import { RECONNECT_MAX_MS, RECONNECT_MIN_MS, SOCKET_URL, WATCHDOG_MS } from "./config";
import type { FromPage, Outcome, Snapshot, ToPage } from "./generated/api";

/** A request as the page makes it, without its number. */
type Request =
  | Omit<Extract<FromPage, { type: "comment" }>, "n">
  | Omit<Extract<FromPage, { type: "settings" }>, "n">;

/** The messages after `hello` that change what the page shows. */
export type Slice = Exclude<ToPage, { type: "hello" } | { type: "reply" } | { type: "ping" }>;

export interface SocketHandlers {
  /** The joiner's whole state, first on every socket. */
  hello(hello: Snapshot): void;
  /** One change, in the joiner's order. */
  slice(slice: Slice): void;
  /** The socket came up (after its hello) or went down. */
  open(open: boolean): void;
}

const LOST: Outcome = { unsent: { error: "jampot join is not reachable" } };

export class PageSocket {
  private socket: WebSocket | null = null;
  /** The hello came: the socket carries the joiner's state. */
  private ready = false;
  private n = 0;
  private readonly waiting = new Map<number, (outcome: Outcome) => void>();
  private lastHeard = Date.now();
  private wait = RECONNECT_MIN_MS;
  private timer: number | undefined;

  constructor(private readonly handlers: SocketHandlers) {
    // The joiner pings every 15 s; a silent socket is a dead one.
    window.setInterval(() => {
      if (this.socket !== null && Date.now() - this.lastHeard > WATCHDOG_MS) this.lost();
    }, 5000);
    // A page kept for back and forward is not one of the joiner's pages
    // meanwhile; it connects again when shown.
    window.addEventListener("pagehide", () => {
      this.drop();
    });
    window.addEventListener("pageshow", (event) => {
      if (event.persisted) this.reconnect(0);
    });
  }

  /** Whether the socket carries the joiner's state now. */
  get isOpen(): boolean {
    return this.ready;
  }

  /** Opens the socket. */
  start(): void {
    this.connect();
  }

  private connect(): void {
    if (this.socket !== null) return;
    const socket = new WebSocket(SOCKET_URL);
    this.socket = socket;
    this.lastHeard = Date.now();
    socket.addEventListener("message", (event: MessageEvent) => {
      if (this.socket === socket) this.receive(parseMessage(event.data));
    });
    socket.addEventListener("close", () => {
      if (this.socket === socket) this.lost();
    });
  }

  private receive(message: ToPage | null): void {
    this.lastHeard = Date.now();
    if (message === null) return;
    switch (message.type) {
      case "hello": {
        this.wait = RECONNECT_MIN_MS;
        this.ready = true;
        this.handlers.hello(message);
        this.handlers.open(true);
        break;
      }
      case "reply": {
        const settle = this.waiting.get(message.to);
        this.waiting.delete(message.to);
        settle?.(message.outcome);
        break;
      }
      case "ping":
        break;
      case "link":
      case "people":
      case "mine":
      case "settings":
      case "reload":
      case "answer":
      case "cursor":
        if (this.ready) this.handlers.slice(message);
        break;
    }
  }

  /** Sends `message` if the socket is up; false if it is not. */
  send(message: FromPage): boolean {
    const socket = this.socket;
    if (!this.ready || socket?.readyState !== WebSocket.OPEN) return false;
    socket.send(JSON.stringify(message));
    return true;
  }

  /** Makes a request; its outcome ("unsent" if the socket goes first). */
  request(request: Request): Promise<Outcome> {
    this.n += 1;
    const n = this.n;
    return new Promise((resolve) => {
      if (!this.send({ ...request, n })) {
        resolve(LOST);
        return;
      }
      this.waiting.set(n, resolve);
    });
  }

  /** Closes the socket without waiting for the joiner to agree. */
  private drop(): void {
    const socket = this.socket;
    this.socket = null;
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.timer = undefined;
    if (socket !== null) socket.close();
    const wasReady = this.ready;
    this.ready = false;
    // Requests on it fail: comments are sent again under their ref.
    for (const settle of this.waiting.values()) settle(LOST);
    this.waiting.clear();
    if (wasReady) this.handlers.open(false);
  }

  /** The socket closed or went silent: shown at once, opened again soon. */
  private lost(): void {
    this.drop();
    this.backOff();
  }

  /** Tries again after the current wait, and doubles it for the next. */
  private backOff(): void {
    this.reconnect(this.wait);
    this.wait = Math.min(this.wait * 2, RECONNECT_MAX_MS);
  }

  private reconnect(wait: number): void {
    if (this.timer !== undefined) return;
    this.timer = window.setTimeout(() => {
      this.timer = undefined;
      this.connect();
    }, wait);
  }
}
