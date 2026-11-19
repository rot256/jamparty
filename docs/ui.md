# jamparty UI (joiner page)

The overlay the joiner's page shows on top of the shared site, as settled in
the design review of 2026-11-12. Mockups: the "jamparty UI" Claude Design
canvas (artboards 1–9; private to the owner), exported here as standalone
HTML with a rendered PNG each: `docs/ui/1-idle` … `docs/ui/9-cursor-options`
(`.html`, `.png`). This file is the source of truth; where it and the
mockups differ, this file wins. The sample page in them stands in for any
shared site; only the overlay is jamparty's.

Everything here lives in the client (`assets/client.js`), the joiner's
local endpoints and the share; nothing in the site itself. The share
injects the client's script tag into each HTML page; the joiner serves
the client itself (`/_jamparty/client.js`, no-store), so a page always runs
its own joiner's client.

## Pill (bottom right)

- One dark pill: the connection dot inside it, left of the text
  `N queued` (`N` = the joiner's own undelivered comments).
- Dot colours: green `#2f9e44` connected, amber `#e8a33d` reconnecting,
  grey `#9a958e` share unreachable or a status line set. Comments not sent
  go out again whenever the joiner is connected to the share, status line
  or not: those made under this joiner's key. Each `jamparty join` has a
  fresh key, never stored, and the share tells comments apart by key and
  ref, so comments from before a joiner started over are never sent again
  by themselves (that would queue them twice); the person may send one
  again by hand.
- The connection dot and pill tooltip show `connected · direct` or
  `connected · relay` (or `mixed` / `unknown` when appropriate). The expanded
  header shows the same state. Only the path label reaches the page, never
  peer IP addresses or relay URLs.
- Clicking the pill expands it upward into the comment list (below);
  clicking again, `▾`, or anywhere outside the list collapses it.
- A gear button sits next to the pill and opens the control panel (below);
  clicking the gear again, or anywhere outside the panel, closes it.
  Only one of the two panels is open at a time.

Comments stay queued until `jamparty recv` prints and confirms them. A nudge
command only reminds the agent and never changes delivery status.

## Comment list (expanded pill)

- Header: dot, `N queued`, `· connected` (or the state), and `▾` to
  collapse.
- The joiner's own comments, newest first. Each row: what it points at
  (the element's short selector, or `page` for a page comment), relative
  time (`3 min ago`), state (`queued`, `delivered` in green, `not sent` in
  red with a `send again` button, `sent earlier` for one queued under a
  joiner that has since started over, whose delivery this one cannot
  follow), then the comment text.
- Only the joiner's own comments; other people's comments are never shown.

## Comment dialog

- Title `Comment:` for element and page comments;
  `Comment on agent response:` stays for replies to answers.
- A reload the dialog did not wait for (a dev server's live reload, or the
  browser's) does not lose the comment: the page opens the dialog again on
  the same target with the text, until it is sent or cancelled. If the
  target is gone, the dialog says "The element is gone; this will be a page
  comment." and sends it as one (after further reloads too). A comment sent
  is the list's from then on (`sending`), never a draft again. While it is
  on its way the dialog stays (Escape and outside clicks wait for the
  joiner's answer); refused, it is the dialog's again, and a draft.

- Use the paperclip, paste files, or drop them into the dialog to attach up
  to four files of up to 8 MiB each. Each chip shows the name and size and
  has a remove button. Comment text is optional while a file is attached.
- Files upload when the comment is submitted. An upload error leaves the
  dialog open with the selected files. After upload, the outbox keeps the
  attachment references with the comment across reconnects and reloads.
  If an attachment is evicted before the comment queues, the page reports
  the failure. Reloading a draft before upload completes asks you to
  reselect the files or explicitly continue without them.

## Edit mode

- An edit button (pen icon) sits between the pill and the gear. Clicking
  it turns it blue (`#2f80ed`) and enters edit mode, as in a design tool.
- In edit mode the element under the pointer is outlined in that blue, and
  a left click comments on it (the `Comment:` dialog, as a right-click
  does) instead of reaching the page: links do not navigate, buttons do not
  fire, nothing gets focus or a text selection. jamparty's own overlay keeps
  working normally.
- It stays on after a comment. Escape leaves it (when a comment dialog is
  open, Escape closes the dialog first), as does clicking the button again.
- Independent of comment mode: the button is always available, and turning
  comment mode off keeps edit mode on. With comment mode off, right-click
  and `/` still reach the page; edit mode takes left clicks only.
- Presses are taken before the page's own listeners (on the window, in the
  capture phase), so delegated handlers on the document do not run either.
- Not saved: every page starts with it off.

## Control panel (gear)

Compact dark panel (about 232 px wide, faint border, light shadow), no hint
text. Rows:

| Row | Control | Default | Key |
|---|---|---|---|
| Comment mode | switch | on (open question, see end) | `C` |
| Show my cursor | switch | **off** | `X` |
| Clear answers | action | dismiss every visible answer | `D` |
| Answer timeout | seconds | **30** (`0` turns it off) | — |
| Corner | four buttons ↖ ↗ ↙ ↘ | bottom right | |

- Comment mode off: the page behaves normally: right-click shows the
  browser menu and `/` types normally. Edit mode can still take left clicks
  to comment on elements. The pill stays.
- Corner: the pill, the gear, both panels, notices and answers not
  anchored to an element move to that corner of the window; panels open
  towards the middle. Kept with the other settings (see Name).
- Show my cursor off: the page sends **no pointer data at all** (not
  "sent and hidden by others"). On: see Cursors.
- Keys work only when no text field, textarea or contenteditable has
  focus, as `/` does. `/` opens a page comment when comment mode is on. A
  held key flips its switch once.
- Rebinding: each key is shown as a chip; clicking a chip puts it in a
  listening state (`press a key…`, dashed outline); the next key press
  becomes the binding. Escape cancels; a modifier (Shift, Control, Alt,
  Meta, Caps Lock) waits for the key pressed with it. A key already bound
  to another action, one jamparty uses (`/`, Escape), or one the page needs
  (Enter, Tab, Space) is refused with a short message instead of
  overwriting. The joiner refuses the same keys, whatever a page sends.
- Answer timeout dismisses each answer after 1–3600 seconds of unpaused
  time; `0` disables it. A thin bar on the bottom shows the time left;
  hover or focus inside the answer pauses it. Reduced motion updates the
  bar once per second without smooth motion. A changed timeout restarts
  visible answers. Timeout and Clear answers
  use the dismiss button's session dismissal behavior. Keys are ignored in
  text fields.
- Settings JSON uses `answerTimeout` (seconds, default 30; 0 turns it
  off) and `keys.clearAnswers` (default `"d"`), alongside
  `keys.commentMode` and `keys.showCursor`.
  `jamparty join --answer-timeout N --clear-answers-key KEY` overrides the
  settings file for a join; `--comment-mode-key` and `--show-cursor-key`
  override the other bindings. All bindings must be distinct, ignoring case.
  Page changes remain in the per-share cookie and never write the
  settings file.
- Bottom row: `N here` on the left, overlapping avatar circles with
  initials on the right (the joiner's own outlined, others filled in their
  colour); the full name on hover. No paths or locations.

## Cursors

- Style: arrow with a name tag hanging off the tip (square at the corner
  nearest the tip), filled in the person's colour, white outline, soft
  shadow; tag in a sans-serif, 11 px, 600 weight. ("A" on artboard 9.)
- Each joiner gets a stable colour derived from their key (a small palette
  with readable white text); the same colour is used for their avatar.
- Shown only to joiners on the same URL. "URL" is the current location,
  not the loaded document: the client hooks `history.pushState`,
  `history.replaceState`, `popstate` and `hashchange`, so single-page apps
  work. Compare path + query + hash.
- Position is sent element-relative, never as raw viewport x/y: the CSS
  selector of the element under the pointer plus the pointer's offset as a
  fraction of that element's box. The receiver draws it only if that
  selector resolves in its own page; otherwise not at all.
- Send only when the pointer moves: one message per `pointermove`,
  throttled to at most about 15 per second (the last position in a window
  is sent, never dropped), and nothing at all while the pointer is still.
  No timers, no periodic or keep-alive cursor messages. Leaving the page
  (`pointerleave`, window blur, tab hidden) sends one `cursor` message
  with no position, so the cursor disappears for others at once.
- The receiver interpolates between positions and fades a cursor after a
  few seconds without an update (its own timer, nothing sent), removing it
  once faded; it removes it at once on the no-position message, when the
  joiner leaves the URL, turns the switch off, or disconnects. It draws
  only the cursors of people its joiner lists as here, one each and at
  most as many as a share takes (64).
- Never shown in the comment dialog's target highlight or captured into
  comments.

## Name

- On first open, if no name is chosen, the page shows a small dialog over
  a dimmed page: "Join the Jam.", one Name field (prefilled with the
  current default), a Join button, and the line "Remembered for this share
  in this browser; change it under the gear." The control panel gains a
  name field to change it later.
- The share's pages run on the joiner's origin, as the overlay does, so
  anything the overlay may do a share's script may do too. Settings
  changed in a page therefore hold for that join only: the joiner never
  writes them to disk, and no other share sees them. The page keeps them
  per share in a host-only cookie on `<sid>.localhost` (cookies ignore the
  port, so a joiner that falls back to another port finds them), holding
  what was changed: the name, the key bindings, the two switch states and
  the corner. When a page loads and its joiner lacks some of them (a
  joiner that started over), it sends them as a change for this join; a
  remembered name keeps the prompt away. A new share is a new origin and
  starts from the defaults.
- The defaults come from `~/.config/jamparty/settings.json` (mode 0600, the
  same fields), which the person writes and jamparty only reads.
  `jamparty join --name` sets the name for one run, until one is chosen in
  the page.
- A name change mid-session is sent to the share and updates presence for
  everyone.

## Addressed replies

- `jamparty send --to <name> "…"`: delivered only to that person's
  connections; nobody else receives it. Matching by name, or by key
  fingerprint when names collide (refuse an ambiguous name with the
  candidates listed).
- `jamparty send --anchor <comment id> "…"` without `--to` addresses the
  comment's author by default; `--to all` (or no anchor and no `--to`)
  sends it to everyone.
- A page shows at most as many answers as its joiner keeps (100), the
  oldest going first, and remembers dismissals only of those.
- The recipient's answer bubble shows a small `to you` line above the text
  (in the answer-meta style, `#7b3221`, 11 px, bold).
- The `jamparty agent` guides describe `--to`.

## Multiple-choice questions

- `jamparty send --choice "Keep it" --choice "Make it bigger" 'Which heading size?'`
  adds buttons below the answer's Markdown. Offer 2–6 distinct, nonblank,
  single-line plain-text labels, at most 80 characters each. The share and
  joiner both enforce the caps; labels render as text, never HTML.
- Audience rules are the same as other answers: without an explicit `--to`,
  an anchor addresses its author; without an anchor it addresses everyone.
- One click submits an ordinary comment with selector
  `jamparty-agent:<answer id>`; `comment` and optional `choice` both hold the
  selected label. The share sets the author and checks the audience and label.
  Picks use the same queue, rate limits and per-key ref deduplication as comments.
- The first valid pick the share can queue wins. The question disappears
  from every page, including the picker's, which shows a brief
  `You chose: <label>` confirmation. A late pick is quietly refused and its
  page removes the question. An unsuccessful delivery can be retried.
- Answered questions are omitted from catch-up, including for the picker.
  Open questions are cleared on disconnect and replayed on admission, so a
  resolution missed during reconnect cannot leave a stale question visible.
  Resolved records share the same 100-entry bound as other answers.
- Right-clicking a question still opens a typed comment. Typed comments do
  not select a choice or resolve the question.

## Protocol (summary)

All of this is part of the version-1 wire format.

Peers must share a release line: `0.x` before 1.0, the major version
thereafter. Prereleases belong to the same line. Every peer supports
choice questions; there is no capability negotiation. The first control
line carries the sender’s jamparty version so a different line can be
refused before parsing the rest. The NDJSON `version` remains 1.

- Agent socket `send` and control `answer` gain optional `choices` labels.
  Control comment drafts and page submissions gain optional `choice`;
  the version-1 comment NDJSON gains only that optional field.
  Control `answer_resolved` and page `answer_resolved` carry the answer id;
  control acknowledgements gain `choice_refused`. Page answers include `choices`.
- Joiner → share: `presence` updates (name change, current URL, cursor on
  or off), `cursor` messages (selector, fractional offset; only while on),
  both rate-limited at the share as well.
- Share → joiner: presence carries name, key fingerprint, colour index and
  current URL per joiner; `cursor` messages forwarded only to joiners on
  the same URL; answers carry an optional recipient key and are sent only
  to that key's connections.

## Tests

- Rust: cursor and presence messages are forwarded only to joiners on the
  same URL; nothing is forwarded for a joiner whose cursor is off (and the
  client sends none); addressed answers reach only the recipient;
  `--anchor` defaults to the author; an ambiguous `--to` is refused; the
  settings file gives the defaults, and a page's change never reaches it
  or another join; key bindings the overlay refuses are refused.
- Browser: **every feature in this file is covered by Playwright**, in
  Chromium and Firefox, against the real binaries (one share, several
  `jamparty join` processes, each with its own home directory and its own
  browser context). A feature is done once its Playwright test passes.
  Single-joiner cases:
  - pill with the dot inside; colour follows the state (stop the share:
    grey; cut the joiner's socket: amber; back: green);
  - expand and collapse; the list shows queued, delivered (after
    `jamparty recv`), and not sent with a working `send again`;
  - gear panel: both switches by click and by key; keys ignored while a
    text field has focus; rebinding (new key works, old one doesn't,
    Escape cancels, a taken key is refused); comment mode off leaves
    right-click and `/` to the page;
  - name prompt on first join; not shown again when the joiner of the
    same share starts over (the page's cookie), shown again for a new
    share; a change under the gear updates presence;
  - the settings file gives the defaults and is never written.

  Multi-client cases (two or three joiners at once, named alice, bob,
  carol):
  - presence: `N here` and the avatars update as joiners connect, rename
    and leave, in every page;
  - cursors: bob turns his on, alice sees bob's cursor and name on the
    same URL and it follows bob's pointer; carol on another URL sees none;
    on a small SPA fixture, a client-side route change by bob makes his
    cursor appear for carol and disappear for alice; bob turns it off:
    it disappears everywhere, and the share receives no further cursor
    messages from him (asserted on the share side);
  - bob's pointer held still sends nothing (the share receives no cursor
    messages over several seconds); a burst of movement stays at or under
    about 15 messages per second and ends on the final position;
  - a cursor anchored to an element that only exists for bob is not drawn
    for alice;
  - comments: alice's and bob's comments each appear only in their own
    list; delivered state reaches only the author;
  - addressed replies: `jamparty send --to alice` shows `to you` for alice
    and nothing for bob; `--anchor` on bob's comment reaches only bob; a
    broadcast reaches everyone;
  - reconnect: one joiner drops and returns; the others' presence and
    cursors update; its comments from before are not sent again by
    themselves (a new joiner has a new key), and go out when sent again
    by hand.

## Open question

- Comment mode default: on (current behaviour) or off (opt in). Keep on
  until decided.

## PDF viewer

Opening a `.pdf` URL in the main browser tab opens the joiner's PDF viewer.
Use the page field to jump to a page and the zoom menu to change its scale.
As with other pages from a share, a complex or hostile PDF can slow down
or crash the viewer tab.
Select text and right-click the selection or press `/` to comment on it.
The pen switches to box drawing. Drag across the part of one page you want
to comment on. The pen stays active until clicked again or you press Escape
with the dialog closed. You can still use `/` for a document comment when
there is no selection.

Your comments in the outbox have blue outlines at their PDF anchors, and
the list labels them by PDF page number. When the PDF changes in a live
share, the viewer keeps the current page, scroll position and zoom. The
outbox and any open dialog stay in place. Box positions refer to the layout
at the time of review and may point to different content after a rebuild.

The usual presence controls apply. In the viewer, cursor positions are
fractions of the PDF page under the pointer, so they follow the content
when other reviewers use a different zoom. Moving outside a page clears
the cursor.
