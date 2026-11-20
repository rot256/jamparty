// Generated from the Rust types by `cargo test` (src/join/api.rs); do not edit.
// Regenerate: JAMPARTY_UPDATE_BINDINGS=1 cargo test bindings

export const MAX_NAME = 40;
export const MAX_ANSWER_TIMEOUT = 3600;
export const DEFAULT_ANSWER_TIMEOUT = 30;
export const MAX_CHOICES = 6;
export const MAX_CHOICE = 80;
export const MAX_COMMENT = 10000;
export const MAX_FILE_BYTES = 8388608;
export const MAX_ATTACHMENTS = 4;
export const MAX_SELECTED_TEXT = 2000;
export const MAX_PAGE_PATH = 512;
export const MAX_CURSOR_SELECTOR = 512;
export const FRACTION_ONE = 10000;
export const MAX_ANSWERS = 100;
export const MAX_JOINERS = 64;
export const DEFAULT_CLEAR_ANSWERS_KEY = "d";
export const RESERVED_KEYS: readonly string[] = ["/", "Escape"];
export const MODIFIER_KEYS: readonly string[] = ["Alt", "AltGraph", "CapsLock", "Control", "Fn", "FnLock", "Hyper", "Meta", "NumLock", "ScrollLock", "Shift", "Super", "Symbol"];
export const PAGE_KEYS: readonly string[] = ["Enter", "Tab", " "];

export type PathKind = "direct" | "relay" | "mixed" | "unknown";

export type CommentId = string;

export type CommentRef = string;

export type CommentPage = string;

export type CommentSelector = string;

export type PdfBox = { x0: number, y0: number, x1: number, y1: number, };

export type PdfAnchorFields = { page: number, box: PdfBox, };

export type Revision = number;

export type EndpointKey = string;

export type Fingerprint = string;

export type JoinerName = string;

export type StatusLine = string;

export type RefusedReason = "wrong_secret" | "bad_hello" | "full" | "incompatible";

export type AnswerTimeout = number;

export type Switch = "on" | "off";

export type Corner = "bottom-right" | "bottom-left" | "top-right" | "top-left";

export type KeyBinding = string;

export type KeysFields = { commentMode: KeyBinding, showCursor: KeyBinding, clearAnswers: KeyBinding, };

export type Keys = { commentMode: KeyBinding, showCursor: KeyBinding, clearAnswers: KeyBinding, };

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
 * No name was chosen yet (none in the settings file or the page, no
 * `--name`): the page asks.
 */
askName: boolean, keys: Keys, commentMode: Switch, showCursor: Switch, corner: Corner, answerTimeout: AnswerTimeout, };

export type SettingsChange = { name?: JoinerName, keys?: Keys, commentMode?: Switch, showCursor?: Switch, corner?: Corner, answerTimeout?: AnswerTimeout, };

export type Choices = Array<string>;

export type Answer = { id: CommentId, html: string, 
/**
 * The ID of the comment it answers.
 */
replyTo?: CommentId, 
/**
 * The selector of the comment it answers.
 */
anchor?: CommentSelector, 
/**
 * The page that comment was made on.
 */
page?: CommentPage, 
/**
 * Addressed to this joiner alone (`jamparty send --to`, or an answer to
 * its comment); the page says so.
 */
toYou: boolean, choices: Choices, 
/**
 * Arrived after the share's admission replay.
 */
live: boolean, };

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
status: StatusLine | null, 
/**
 * Locally observed selected transport, never supplied by the share.
 */
path: PathKind | null, };

export type Snapshot = { link: Link, 
/**
 * Who is here, this joiner included, with their colour, page and
 * cursor switch.
 */
people: Array<Person>, 
/**
 * This joiner's comments the share holds undelivered; none until the
 * share first said.
 */
mine: Array<CommentId> | null, settings: PageSettings, 
/**
 * Recent answers, oldest first: at most [`MAX_ANSWERS`], of at most
 * [`MAX_ANSWERS_HTML`] together.
 */
answers: Array<Answer>, 
/**
 * The live-reload revision (shared directories only).
 */
revision: Revision | null, };

export type RequestNumber = number;

export type CommentOutcome = { "commented": { id: CommentId, } } | { "unsent": { error: string, } } | { "refused": { error: string, choiceRefused?: boolean, attachmentUnavailable?: boolean, retryAfterSecs?: number, } };

export type SettingsOutcome = "saved" | { "refused": { error: string, } };

export type ToPage = { "type": "hello" } & Snapshot | { "type": "link" } & Link | { "type": "people", here: Array<Person>, } | { "type": "mine", undelivered: Array<CommentId> | null, } | { "type": "settings" } & PageSettings | { "type": "reload", revision: Revision, } | { "type": "answer" } & Answer | { "type": "answer_resolved", id: CommentId, } | { "type": "cursor", key: EndpointKey, url: PagePath, position: Position | null, } | { "type": "comment_reply", to: RequestNumber, outcome: CommentOutcome, } | { "type": "settings_reply", to: RequestNumber, outcome: SettingsOutcome, } | { "type": "ping" };

export type ResourceId = string;

export type AttachmentName = string;

export type Attachment = { "type": "file", file: ResourceId, name: AttachmentName, };

export type Submission = { n: RequestNumber, ref: CommentRef, page: CommentPage, selector: CommentSelector, pdf?: PdfAnchorFields, selectedText: string | null, choice?: string, comment: string, attachments?: Array<Attachment>, };

export type Change = { n: RequestNumber, change: SettingsChange, };

export type FromPage = { "type": "page", url: PagePath | null, } | { "type": "cursor", position: Position | null, } | { "type": "comment" } & Submission | { "type": "settings" } & Change;
