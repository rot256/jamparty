// Generated from the Rust types by `cargo test` (src/join/api.rs); do not edit.
// Regenerate: JAMPOT_UPDATE_BINDINGS=1 cargo test bindings

export const API_VERSION = 2;

export type CommentId = string;

export type CommentRef = string;

export type EndpointKey = string;

export type Fingerprint = string;

export type JoinerName = string;

export type StatusLine = string;

export type RefusedReason = "wrong_secret" | "bad_hello" | "full";

export type Switch = "on" | "off";

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

export type TabId = string;

export type Visit = { tab: TabId, url: PagePath, };

export type PageSettings = { 
/**
 * The name this joiner goes by now.
 */
name: JoinerName, 
/**
 * No name was chosen yet (none saved, no `--name`): the page asks.
 */
askName: boolean, keys: Keys, commentMode: Switch, showCursor: Switch, };

export type SettingsChange = { name?: JoinerName, keys?: Keys, commentMode?: Switch, showCursor?: Switch, };

export type Answer = { id: CommentId, html: string, 
/**
 * The selector of the comment it answers.
 */
anchor?: string, 
/**
 * The page that comment was made on.
 */
page?: string, };

export type View = { 
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
 * Who is connected, this joiner included.
 */
joiners: Array<Author>, 
/**
 * The same people with their colour, page and cursor switch.
 */
here: Array<Person>, 
/**
 * The live-reload revision (shared directories only).
 */
revision: number | null, status: StatusLine | null, 
/**
 * This joiner's comments the share holds undelivered.
 */
pending: number, 
/**
 * Their ids.
 */
undelivered: Array<CommentId>, 
/**
 * Recent answers, oldest first.
 */
answers: Array<Answer>, settings: PageSettings, };

export type Status = { api: number, version: string, 
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
 * Who is connected, this joiner included.
 */
joiners: Array<Author>, 
/**
 * The same people with their colour, page and cursor switch.
 */
here: Array<Person>, 
/**
 * The live-reload revision (shared directories only).
 */
revision: number | null, status: StatusLine | null, 
/**
 * This joiner's comments the share holds undelivered.
 */
pending: number, 
/**
 * Their ids.
 */
undelivered: Array<CommentId>, 
/**
 * Recent answers, oldest first.
 */
answers: Array<Answer>, settings: PageSettings, };

export type Event = { "type": "share", connected: boolean, } | { "type": "reload", revision: number, } | { "type": "presence", joiners: Array<Author>, here: Array<Person>, } | { "type": "status", line: StatusLine | null, } | { "type": "delivered", ids: Array<CommentId>, } | { "type": "answer" } & Answer | { "type": "settings" } & PageSettings;

export type Submission = { ref: CommentRef, page: string, selector: string, selectedText: string | null, comment: string, };

export type Queued = { id: CommentId, pending: number, };

export type Limited429 = { error: string, retryAfterSecs: number, };

export type ErrorBody = { error: string, };
