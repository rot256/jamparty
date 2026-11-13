// The joiner's page API: `api/status`, which says which version of it the
// joiner speaks, and the messages on its socket, typed by the bindings
// generated from its Rust types. The joiner may run another jampot than
// the share this client came from, so the version is checked before the
// socket is opened, and again in its hello.

import { SHARE_VERSION, STATUS_URL } from "./config";
import { API_VERSION, type ToPage } from "./generated/api";

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** Why the joiner cannot serve this page, or null when it can. */
export function incompatibility(api: unknown, version: unknown): string | null {
  if (api === API_VERSION) return null;
  const share = SHARE_VERSION ?? "unknown";
  const joiner = typeof version === "string" ? version : "another";
  return typeof api === "number"
    ? `jampot versions do not match: the share runs ${share} (page API ${API_VERSION}), ` +
      `your jampot join runs ${joiner} (page API ${api}). ` +
      "Use the same jampot version on both sides and join again."
    : `Your jampot join is older than the share's jampot ${share} and cannot serve this page. ` +
      "Use the same jampot version on both sides and join again.";
}

/**
 * Asks the joiner which page API it speaks: null when it is not
 * reachable, "" when it speaks this one, or why it cannot serve the page.
 */
export async function probe(): Promise<string | null> {
  try {
    const response = await fetch(STATUS_URL, { cache: "no-store" });
    if (!response.ok) return null;
    const body: unknown = await response.json();
    return incompatibility(
      isObject(body) ? body.api : undefined,
      isObject(body) ? body.version : undefined,
    ) ?? "";
  } catch {
    return null;
  }
}

/** A socket message, if `data` is one (unknown types pass: they are ignored). */
export function parseMessage(data: unknown): ToPage | null {
  if (typeof data !== "string") return null;
  try {
    const message: unknown = JSON.parse(data);
    return isObject(message) && typeof message.type === "string" ? (message as ToPage) : null;
  } catch {
    return null;
  }
}
