// The joiner's page API: the messages on its socket, typed by the
// bindings generated from its Rust types. The joiner serves this client
// too; a page that outlived its joiner (restarted as another jampot)
// learns so from the version in the hello.

import { API_VERSION, type ToPage } from "./generated/api";
import { isRecord } from "./json";

/** Why the joiner cannot serve this page, or null when it can. */
export function incompatibility(api: number, version: string): string | null {
  if (api === API_VERSION) return null;
  return `This page's jampot client (page API ${API_VERSION}) does not match ` +
    `jampot join ${version} (page API ${api}). Reload the page.`;
}

/** A socket message, if `data` is one (unknown types pass: they are ignored). */
export function parseMessage(data: unknown): ToPage | null {
  if (typeof data !== "string") return null;
  try {
    const message: unknown = JSON.parse(data);
    return isRecord(message) && typeof message.type === "string" ? (message as ToPage) : null;
  } catch {
    return null;
  }
}
