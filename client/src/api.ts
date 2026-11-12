// The joiner's endpoints (`/_jampot/api/*`), typed by the bindings
// generated from its Rust types. The joiner may run another jampot than
// the share this client came from, so `api/status` is checked for the
// API version before anything else is trusted.

import {
  COMMENTS_URL,
  CURSOR_URL,
  EVENTS_URL,
  PAGE_URL,
  SETTINGS_URL,
  SHARE_VERSION,
  STATUS_URL,
} from "./config";
import {
  API_VERSION,
  type Event,
  type PageSettings,
  type Pointer,
  type Queued,
  type SettingsChange,
  type Status,
  type Submission,
  type Visit,
} from "./generated/api";

/** What `api/events` sends: the joiner's events and its keep-alive. */
export type ServerEvent = Event | { type: "ping" };

/** A failed submission; `unsent` when the comment may not have reached the share. */
export class SubmitError extends Error {
  constructor(message: string, readonly unsent: boolean) {
    super(message);
  }
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function errorMessage(body: unknown, fallback: string): string {
  return isObject(body) && typeof body.error === "string" ? body.error : fallback;
}

/**
 * Hands a comment to the joiner. Sending the same `ref` again never
 * queues it twice.
 */
export async function submit(submission: Submission): Promise<Queued> {
  let response: Response;
  try {
    response = await fetch(COMMENTS_URL, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(submission),
    });
  } catch {
    throw new SubmitError("jampot join is not reachable", true);
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) {
    throw new SubmitError(
      errorMessage(body, `request failed (${response.status})`),
      response.status === 503,
    );
  }
  return body as Queued;
}

export type Checked = { status: Status } | { incompatible: string };

/** `api/status`, checked for the API version; null when the joiner is not reachable. */
export async function fetchStatus(): Promise<Checked | null> {
  try {
    const response = await fetch(STATUS_URL);
    if (!response.ok) return null;
    return checkVersion(await response.json());
  } catch {
    return null;
  }
}

export function checkVersion(body: unknown): Checked {
  const api = isObject(body) ? body.api : undefined;
  if (api === API_VERSION) return { status: body as Status };
  const share = SHARE_VERSION ?? "unknown";
  const joiner = isObject(body) && typeof body.version === "string" ? body.version : "another";
  const incompatible = typeof api === "number"
    ? `jampot versions do not match: the share runs ${share} (page API ${API_VERSION}), ` +
      `your jampot join runs ${joiner} (page API ${api}). ` +
      "Use the same jampot version on both sides and join again."
    : `Your jampot join is older than the share's jampot ${share} and cannot serve this page. ` +
      "Use the same jampot version on both sides and join again.";
  return { incompatible };
}

/** The event stream; `tab` and `url` make this page one of the joiner's tabs. */
export function openEvents(tab: { tab: string; url: string } | null): EventSource {
  const url = new URL(EVENTS_URL);
  if (tab !== null) {
    url.searchParams.set("tab", tab.tab);
    url.searchParams.set("url", tab.url);
  }
  return new EventSource(url);
}

async function post(url: URL, body: unknown): Promise<Response> {
  return fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

/** Changes the joiner's settings; the settings as they are now, or why not. */
export async function changeSettings(change: SettingsChange): Promise<PageSettings> {
  let response: Response;
  try {
    response = await post(SETTINGS_URL, change);
  } catch {
    throw new Error("jampot join is not reachable");
  }
  const body: unknown = await response.json().catch(() => null);
  if (!response.ok) throw new Error(errorMessage(body, `request failed (${response.status})`));
  return body as PageSettings;
}

/** Tells the joiner this tab is on `visit.url` now; false if it did not take it. */
export async function reportPage(visit: Visit): Promise<boolean> {
  try {
    return (await post(PAGE_URL, visit)).ok;
  } catch {
    return false;
  }
}

/** Hands the pointer to the joiner (no position: it left the page). */
export async function sendCursor(pointer: Pointer): Promise<void> {
  try {
    await post(CURSOR_URL, pointer);
  } catch {
    // A lost cursor update is replaced by the next movement.
  }
}

export function parseEvent(data: string): ServerEvent | null {
  try {
    const event: unknown = JSON.parse(data);
    return isObject(event) && typeof event.type === "string" ? (event as ServerEvent) : null;
  } catch {
    return null;
  }
}
