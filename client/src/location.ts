// Where this page is: its URL as the share compares it (path, query and
// hash; hosts differ per joiner), followed through single-page-app route
// changes.

import { MAX_PAGE_PATH } from "./generated/api";

/**
 * The page's URL without its origin, in the joiner's alphabet (printable
 * ASCII, no `"` or `\`); null when it cannot say (too long).
 */
export function pageUrl(): string | null {
  const url = new URL(window.location.href);
  const raw = `${url.pathname}${url.search}${url.hash}`;
  const encoded = raw.replace(/[^\x21-\x7e]|["\\]/gu, (character) => encodeURIComponent(character));
  return encoded.startsWith("/") && encoded.length <= MAX_PAGE_PATH ? encoded : null;
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
