// Generated from the Rust types by `cargo test` (src/join/api.rs); do not edit.
// Regenerate: JAMPOT_UPDATE_BINDINGS=1 cargo test bindings

export const API_VERSION = 1;

export type CommentId = string;

export type CommentRef = string;

export type EndpointKey = string;

export type Fingerprint = string;

export type JoinerName = string;

export type StatusLine = string;

export type RefusedReason = "wrong_secret" | "bad_hello" | "full";

export type Author = { name: JoinerName, fingerprint: Fingerprint, key: EndpointKey, };

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
answers: Array<Answer>, };

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
answers: Array<Answer>, };

export type Event = { "type": "share", connected: boolean, } | { "type": "reload", revision: number, } | { "type": "presence", joiners: Array<Author>, } | { "type": "status", line: StatusLine | null, } | { "type": "delivered", ids: Array<CommentId>, } | { "type": "answer" } & Answer;

export type Submission = { ref: CommentRef, page: string, selector: string, selectedText: string | null, comment: string, };

export type Queued = { id: CommentId, pending: number, };

export type Limited429 = { error: string, retryAfterSecs: number, };

export type ErrorBody = { error: string, };
