// Where the client talks to, and what the share told it in the script URL.

const SCRIPT_URL = new URL(import.meta.url);
const BASE = new URL("./", SCRIPT_URL);

export const COMMENTS_URL = new URL("api/comments", BASE);
export const STATUS_URL = new URL("api/status", BASE);
export const EVENTS_URL = new URL("api/events", BASE);

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
