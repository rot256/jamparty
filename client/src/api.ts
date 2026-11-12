// The joiner's endpoints (`/_jampot/api/*`), typed by the bindings
// generated from its Rust types. The joiner may run another jampot than
// the share this client came from, so `api/status` is checked for the
// API version before anything else is trusted.

import { COMMENTS_URL, EVENTS_URL, SHARE_VERSION, STATUS_URL } from "./config";
import {
  API_VERSION,
  type Event,
  type Queued,
  type Status,
  type Submission,
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

export function openEvents(): EventSource {
  return new EventSource(EVENTS_URL);
}

export function parseEvent(data: string): ServerEvent | null {
  try {
    const event: unknown = JSON.parse(data);
    return isObject(event) && typeof event.type === "string" ? (event as ServerEvent) : null;
  } catch {
    return null;
  }
}
