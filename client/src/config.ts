// Where the client talks to, and what the share told it in the script URL.

const SCRIPT_URL = new URL(import.meta.url);
const BASE = new URL("./", SCRIPT_URL);

export const COMMENTS_URL = new URL("api/comments", BASE);
export const STATUS_URL = new URL("api/status", BASE);
export const EVENTS_URL = new URL("api/events", BASE);
export const SETTINGS_URL = new URL("api/settings", BASE);
export const PAGE_URL = new URL("api/page", BASE);
export const CURSOR_URL = new URL("api/cursor", BASE);

/** The live-reload revision this page was served at (shared directories only). */
export const LIVE_REVISION = SCRIPT_URL.searchParams.get("live");
/** The share's jampot version: this client came from it. */
export const SHARE_VERSION = SCRIPT_URL.searchParams.get("version");

export const HOST_ID = "jampot";
export const MAX_SELECTED_TEXT = 2000;
export const DISMISSED_KEY = "jampot-dismissed-v1";
export const OUTBOX_KEY = "jampot-outbox-v1";
export const MAX_OUTBOX = 20;
/** The joiner pings every 15 s; a stream silent for longer than this is dead. */
export const WATCHDOG_MS = 35000;
/** Selectors of agent answers, for comments on an answer. */
export const AGENT_TARGET_PREFIX = "jampot-agent:";
/** Cursor messages per second at most (docs/ui.md: about 15). */
export const CURSOR_RATE = 15;
/** A cursor without an update for this long fades out. */
export const CURSOR_FADE_MS = 4000;
/** Keys jampot keeps for itself (`/` opens a page comment, Escape cancels). */
export const RESERVED_KEYS = ["/", "Escape"];
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
