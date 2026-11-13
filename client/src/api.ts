// The joiner's page API: the messages on its socket, typed by the
// bindings generated from its Rust types. The joiner serves this client
// too, so the two always match.

import type { ToPage } from "./generated/api";
import { isRecord } from "./json";

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
