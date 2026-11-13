// Generated from the Rust types by `cargo test` (src/join/api.rs); do not edit.
// Regenerate: JAMPOT_UPDATE_BINDINGS=1 cargo test bindings

export const MAX_NAME = 40;
export const MAX_COMMENT = 10000;
export const MAX_SELECTED_TEXT = 2000;
export const MAX_PAGE_PATH = 512;
export const MAX_CURSOR_SELECTOR = 512;
export const FRACTION_ONE = 10000;
export const RESERVED_KEYS: readonly string[] = ["/", "Escape"];

export type CommentId = string;

export type CommentRef = string;

export type CommentPage = string;

export type CommentSelector = string;

export type Revision = number;

export type EndpointKey = string;

export type Fingerprint = string;

export type JoinerName = string;

export type StatusLine = string;

export type RefusedReason = "wrong_secret" | "bad_hello" | "full" | "incompatible";

export type Switch = "on" | "off";

export type Corner = "bottom-right" | "bottom-left" | "top-right" | "top-left";

export type KeyBinding = string;

export type KeysFields = { commentMode: KeyBinding, showCursor: KeyBinding, };

export type Keys = { commentMode: KeyBinding, showCursor: KeyBinding, };

export type Author = { name: JoinerName, fingerprint: Fingerprint, key: EndpointKey, };

export type PagePath = string;

export type Colour = number;

export type Person = { name: JoinerName, fingerprint: Fingerprint, key: EndpointKey, colour: Colour, 
/**
 * The page they are on, once their page said.
 */
url: PagePath | null, cursor: Switch, };

export type CursorSelector = string;

export type Fraction = number;

export type Position = { selector: CursorSelector, x: Fraction, y: Fraction, };

export type PageSettings = { 
/**
 * The name this joiner goes by now.
 */
name: JoinerName, 
/**
 * No name was chosen yet (none saved, no `--name`): the page asks.
 */
askName: boolean, keys: Keys, commentMode: Switch, showCursor: Switch, corner: Corner, };

export type SettingsChange = { name?: JoinerName, keys?: Keys, commentMode?: Switch, showCursor?: Switch, corner?: Corner, };

export type Answer = { id: CommentId, html: string, 
/**
 * The selector of the comment it answers.
 */
anchor?: CommentSelector, 
/**
 * The page that comment was made on.
 */
page?: CommentPage, 
/**
 * Addressed to this joiner alone (`jampot send --to`, or an answer to
 * its comment); the page says so.
 */
toYou: boolean, };

export type Link = { 
/**
 * Connected to the share and admitted.
 */
connected: boolean, 
/**
 * Set when the share refused this joiner; it does not retry then.
 */
refused: RefusedReason | null, 
/**
 * How this joiner's comments are signed.
 */
you: Author | null, 
/**
 * The share's status line.
 */
status: StatusLine | null, };

export type Snapshot = { link: Link, 
/**
 * Who is here, this joiner included, with their colour, page and
 * cursor switch.
 */
people: Array<Person>, 
/**
 * This joiner's comments the share holds undelivered; none until the
 * share first said (a joiner that restarted does not know yet).
 */
mine: Array<CommentId> | null, settings: PageSettings, 
/**
 * Recent answers, oldest first.
 */
answers: Array<Answer>, 
/**
 * The live-reload revision (shared directories only).
 */
revision: Revision | null, };

export type RequestNumber = number;

export type CommentOutcome = { "commented": { id: CommentId, } } | { "unsent": { error: string, } } | { "refused": { error: string, retryAfterSecs?: number, } };

export type SettingsOutcome = "saved" | { "refused": { error: string, } };

export type ToPage = { "type": "hello" } & Snapshot | { "type": "link" } & Link | { "type": "people", here: Array<Person>, } | { "type": "mine", undelivered: Array<CommentId> | null, } | { "type": "settings" } & PageSettings | { "type": "reload", revision: Revision, } | { "type": "answer" } & Answer | { "type": "cursor", key: EndpointKey, url: PagePath, position: Position | null, } | { "type": "comment_reply", to: RequestNumber, outcome: CommentOutcome, } | { "type": "settings_reply", to: RequestNumber, outcome: SettingsOutcome, } | { "type": "ping" };

export type Submission = { n: RequestNumber, ref: CommentRef, page: CommentPage, selector: CommentSelector, selectedText: string | null, comment: string, };

export type Change = { n: RequestNumber, change: SettingsChange, };

export type FromPage = { "type": "page", url: PagePath | null, } | { "type": "cursor", position: Position | null, } | { "type": "comment" } & Submission | { "type": "settings" } & Change;
