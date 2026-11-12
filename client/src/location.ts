// Where this page is: its URL as the share compares it (path, query and
// hash; hosts differ per joiner), followed through single-page-app route
// changes, and this tab's id for the joiner.

import { reportPage } from "./api";

/** This tab, for the joiner (it tracks which URL each tab is on). */
export const TAB_ID = crypto.randomUUID();

const MAX_URL = 512;

/**
 * The page's URL without its origin, in the joiner's alphabet (printable
 * ASCII, no `"` or `\`); null when it cannot say (too long).
 */
export function pageUrl(): string | null {
  const url = new URL(window.location.href);
  const raw = `${url.pathname}${url.search}${url.hash}`;
  const encoded = raw.replace(/[^\x21-\x7e]|["\\]/gu, (character) => encodeURIComponent(character));
  return encoded.startsWith("/") && encoded.length <= MAX_URL ? encoded : null;
}

type Listener = (url: string | null) => void;

/**
 * Calls `listener` whenever the URL changes: history.pushState and
 * replaceState (wrapped), popstate and hashchange.
 */
export function watchUrl(listener: Listener): void {
  let last = pageUrl();
  const check = (): void => {
    const now = pageUrl();
    if (now === last) return;
    last = now;
    listener(now);
  };
  for (const method of ["pushState", "replaceState"] as const) {
    const original = history[method].bind(history);
    history[method] = (...args: Parameters<History["pushState"]>): void => {
      original(...args);
      check();
    };
  }
  window.addEventListener("popstate", check);
  window.addEventListener("hashchange", check);
}

/**
 * This tab's page reports to the joiner: one request at a time, and only
 * the latest page waits behind it. Each report is numbered, counting up,
 * so the joiner ignores one that arrives after a newer one.
 */
export class Registration {
  private seq = 0;
  private sending = false;
  /** The page waiting to be reported (undefined: none). */
  private wanted: { url: string | null } | undefined;
  /** Called when the joiner took the latest report. */
  onConfirmed: (url: string | null, seq: number) => void = () => undefined;

  /** The number of the latest report made. */
  get latest(): number {
    return this.seq;
  }

  request(url: string | null): void {
    this.wanted = { url };
    void this.pump();
  }

  /** Whether a page came in while a report was out. */
  private newerWaiting(): boolean {
    return this.wanted !== undefined;
  }

  private async pump(): Promise<void> {
    if (this.sending) return;
    this.sending = true;
    for (let next = this.wanted; next !== undefined; next = this.wanted) {
      this.wanted = undefined;
      this.seq += 1;
      const seq = this.seq;
      const taken = await reportPage({ tab: TAB_ID, seq, url: next.url });
      // A newer page waits: that one is confirmed instead.
      if (taken && !this.newerWaiting()) this.onConfirmed(next.url, seq);
    }
    this.sending = false;
  }
}
