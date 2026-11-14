// Where the client talks to, and what the share told it in the script URL.

const SCRIPT_URL = new URL(import.meta.url);
const BASE = new URL("./", SCRIPT_URL);

/** The joiner's socket for this page, on the same host. */
export const SOCKET_URL = ((): URL => {
  const url = new URL("api/socket", BASE);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url;
})();

/** The live-reload revision this page was served at (shared directories only). */
export const LIVE_REVISION = ((): number | null => {
  const live = SCRIPT_URL.searchParams.get("live");
  if (live === null || !/^\d+$/.test(live)) return null;
  const revision = Number(live);
  return Number.isSafeInteger(revision) ? revision : null;
})();

export const HOST_ID = "jampot";
export const DISMISSED_KEY = "jampot-dismissed-v1";
export const OUTBOX_KEY = "jampot-outbox-v1";
export const DRAFT_KEY = "jampot-draft-v1";
/** The cookie that keeps the settings changed in this share's pages. */
export const SETTINGS_COOKIE = "jampot-settings-v1";
export const MAX_OUTBOX = 20;
/** The joiner pings every 15 s; a socket silent for longer than this is dead. */
export const WATCHDOG_MS = 35000;
/** The first wait before reconnecting to the joiner, doubled up to the most. */
export const RECONNECT_MIN_MS = 250;
export const RECONNECT_MAX_MS = 5000;
/** Selectors of agent answers, for comments on an answer. */
export const AGENT_TARGET_PREFIX = "jampot-agent:";
/** Cursor messages per second at most (docs/ui.md: about 15). */
export const CURSOR_RATE = 15;
/** A cursor without an update for this long fades out. */
export const CURSOR_FADE_MS = 4000;
/** The colours people are shown in, by the share's colour index; white text reads on each. */
export const PALETTE = [
  "#0b7896",
  "#9c4f96",
  "#b4461f",
  "#2b7a3d",
  "#5f5bb8",
  "#a33a5b",
  "#7a6200",
  "#3d6b8f",
];
