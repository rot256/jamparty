// The settings changed in this share's pages, kept in a cookie of the
// share's origin. The joiner takes a page's change for its current join
// only and never writes it to disk; the page keeps it here and hands it
// over again when a joiner of this share starts over. The cookie is
// host-only (no Domain), so it stays with `<sid>.localhost`, whatever
// port the joiner serves on.

import { SETTINGS_COOKIE } from "./config";
import {
  type Corner,
  MAX_ANSWER_TIMEOUT,
  type PageSettings,
  type SettingsChange,
  type Switch,
} from "./generated/api";
import { isRecord } from "./json";

/** Kept for a year; a share's origin lasts as long as the share. */
const MAX_AGE_SECS = 365 * 24 * 60 * 60;

function isSwitch(value: unknown): value is Switch {
  return value === "on" || value === "off";
}

function isCorner(value: unknown): value is Corner {
  return value === "bottom-right" || value === "bottom-left" ||
    value === "top-right" || value === "top-left";
}

/** The fields of `value` that look like a settings change's; the joiner checks the rest. */
function parse(value: unknown): SettingsChange {
  if (!isRecord(value)) return {};
  const change: SettingsChange = {};
  const { name, keys, commentMode, showCursor, corner, answerTimeout } = value;
  if (typeof name === "string") change.name = name;
  if (
    isRecord(keys) && typeof keys.commentMode === "string" &&
    typeof keys.showCursor === "string" && typeof keys.clearAnswers === "string"
  ) {
    change.keys = { commentMode: keys.commentMode, showCursor: keys.showCursor,
      clearAnswers: keys.clearAnswers };
  }
  if (isSwitch(commentMode)) change.commentMode = commentMode;
  if (isSwitch(showCursor)) change.showCursor = showCursor;
  if (typeof answerTimeout === "number" && Number.isInteger(answerTimeout) &&
      answerTimeout >= 0 && answerTimeout <= MAX_ANSWER_TIMEOUT) change.answerTimeout = answerTimeout;
  if (isCorner(corner)) change.corner = corner;
  return change;
}

/** What this share's pages changed, as one change. */
export function remembered(): SettingsChange {
  const prefix = `${SETTINGS_COOKIE}=`;
  const cookie = document.cookie
    .split("; ")
    .find((part) => part.startsWith(prefix));
  if (cookie === undefined) return {};
  try {
    return parse(JSON.parse(decodeURIComponent(cookie.slice(prefix.length))));
  } catch {
    return {};
  }
}

/** Replaces what this share's pages changed with `change`. */
export function keep(change: SettingsChange): void {
  const value = encodeURIComponent(JSON.stringify(change));
  document.cookie = `${SETTINGS_COOKIE}=${value}; path=/; max-age=${MAX_AGE_SECS}; samesite=strict`;
}

/** Adds a change to what this share's pages changed. */
export function remember(change: SettingsChange): void {
  keep({ ...remembered(), ...parse(change) });
}

/** Forgets what this share's pages changed (the joiner refused it). */
export function forget(): void {
  document.cookie = `${SETTINGS_COOKIE}=; path=/; max-age=0; samesite=strict`;
}

/**
 * The part of `change` that `settings` do not have yet; null when they
 * have all of it.
 */
export function missing(change: SettingsChange, settings: PageSettings): SettingsChange | null {
  const needed: SettingsChange = {};
  let any = false;
  if (change.name !== undefined && (change.name !== settings.name || settings.askName)) {
    needed.name = change.name;
    any = true;
  }
  if (
    change.keys !== undefined && (
      change.keys.commentMode !== settings.keys.commentMode ||
      change.keys.showCursor !== settings.keys.showCursor ||
      change.keys.clearAnswers !== settings.keys.clearAnswers
    )
  ) {
    needed.keys = change.keys;
    any = true;
  }
  if (change.commentMode !== undefined && change.commentMode !== settings.commentMode) {
    needed.commentMode = change.commentMode;
    any = true;
  }
  if (change.showCursor !== undefined && change.showCursor !== settings.showCursor) {
    needed.showCursor = change.showCursor;
    any = true;
  }
  if (change.answerTimeout !== undefined && change.answerTimeout !== settings.answerTimeout) {
    needed.answerTimeout = change.answerTimeout;
    any = true;
  }
  if (change.corner !== undefined && change.corner !== settings.corner) {
    needed.corner = change.corner;
    any = true;
  }
  return any ? needed : null;
}
