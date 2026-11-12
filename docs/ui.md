# jampot UI (joiner page)

The overlay the joiner's page shows on top of the shared site, as settled in
the design review of 2026-11-12. Mockups: the "jampot UI" Claude Design
canvas (artboards 1–9; private to the owner), exported here as standalone
HTML with a rendered PNG each: `docs/ui/1-idle` … `docs/ui/9-cursor-options`
(`.html`, `.png`). This file is the source of truth; where it and the
mockups differ, this file wins. The sample page in them stands in for any
shared site; only the overlay is jampot's.

Everything here lives in the injected client (`assets/client.js`), the
joiner's local endpoints and the share; nothing in the site itself.

## Pill (bottom right)

- One dark pill: the connection dot inside it, left of the text
  `N queued` (`N` = the joiner's own undelivered comments). The separate
  dot is gone.
- Dot colours as today: green `#2f9e44` connected, amber `#e8a33d`
  reconnecting, grey `#9a958e` share unreachable or a status line set.
- Clicking the pill expands it upward into the comment list (below);
  clicking again or `▾` collapses it.
- A gear button sits next to the pill and opens the control panel (below).
  Only one of the two panels is open at a time.

## Comment list (expanded pill)

- Header: dot, `N queued`, `· connected` (or the state), and `▾` to
  collapse.
- The joiner's own comments, newest first. Each row: what it points at
  (the element's short selector, or `page` for a page comment), relative
  time (`3 min ago`), state (`queued`, `delivered` in green, `not sent` in
  red with a `send again` button), then the comment text.
- Only the joiner's own comments; other people's comments are never shown.

## Comment dialog

- Title `Comment:` for element and page comments (done, `d4b01b4`);
  `Comment on agent response:` stays for replies to answers.

## Control panel (gear)

Compact dark panel (about 232 px wide, faint border, light shadow), no hint
text. Rows:

| Row | Control | Default | Key |
|---|---|---|---|
| Comment mode | switch | on (open question, see end) | `C` |
| Show my cursor | switch | **off** | `X` |

- Comment mode off: the page behaves normally: right-click shows the
  browser menu, `/` types normally, nothing is intercepted. The pill stays.
- Show my cursor off: the page sends **no pointer data at all** (not
  "sent and hidden by others"). On: see Cursors.
- Keys work only when no text field, textarea or contenteditable has
  focus (as `/` today). `/` still opens a page comment when comment mode is
  on.
- Rebinding: each key is shown as a chip; clicking a chip puts it in a
  listening state (`press a key…`, dashed outline); the next key press
  becomes the binding. Escape cancels. A key already bound to another
  action, or one jampot uses (`/`, Escape), is refused with a short message
  instead of overwriting.
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
- Throttle sending to about 15 updates per second; the receiver
  interpolates. Fade a cursor after a few seconds without movement; remove
  it when the joiner leaves the URL, turns the switch off, or disconnects.
- Never shown in the comment dialog's target highlight or captured into
  comments.

## Name

- On first open, if no name is saved, the page shows a small dialog over a
  dimmed page: "Join the potluck.", one Name field (prefilled with the
  current default), a Join button, and the line "Remembered on this
  computer; change it under the gear." The control panel gains a name field
  to change it later.
- Stored by the joiner process on disk, not in a cookie (each session is a
  different `<sid>.localhost` origin, so a cookie would be lost per share):
  `~/.config/jampot/settings.json` next to the key, mode 0600, holding the
  name and the key bindings (and the two switch states).
  `jampot join --name` still overrides the name for one run.
- A name change mid-session is sent to the share and updates presence for
  everyone.

## Addressed replies

- `jampot send --to <name> "…"`: delivered only to that person's
  connections; nobody else receives it. Matching by name, or by key
  fingerprint when names collide (refuse an ambiguous name with the
  candidates listed).
- `jampot send --anchor <comment id> "…"` without `--to` addresses the
  comment's author by default; `--to all` (or no anchor and no `--to`)
  broadcasts as today.
- The recipient's answer bubble shows a small `to you` line above the text
  (in the existing answer-meta style, `#7b3221`, 11 px, bold).
- Update `jampot agent` guides to describe `--to`.

## Protocol and share changes (summary)

- Joiner → share: `presence` updates (name change, current URL, cursor on
  or off), `cursor` messages (selector, fractional offset; only while on),
  both rate-limited at the share as well.
- Share → joiner: presence carries name, key fingerprint, colour index and
  current URL per joiner; `cursor` messages forwarded only to joiners on
  the same URL; answers gain an optional recipient key and are sent only
  to that key's connections.
- Bump the wire `version` if the changes are not purely additive.

## Tests to add

- Rust: cursor and presence messages are forwarded only to joiners on the
  same URL; nothing is forwarded for a joiner whose cursor is off (and the
  client sends none); addressed answers reach only the recipient;
  `--anchor` defaults to the author; ambiguous `--to` is refused; settings
  file written 0600 and read back.
- Browser: **every feature in this file is covered by Playwright**, in
  Chromium and Firefox, against the real binaries (one share, several
  `jampot join` processes, each with its own home directory and its own
  browser context). A feature is not done until its Playwright test passes.
  Single-joiner cases:
  - pill with the dot inside; colour follows the state (stop the share:
    grey; cut the joiner's event stream: amber; back: green);
  - expand and collapse; the list shows queued, delivered (after
    `jampot recv`), and not sent with a working `send again`;
  - gear panel: both switches by click and by key; keys ignored while a
    text field has focus; rebinding (new key works, old one doesn't,
    Escape cancels, a taken key is refused); comment mode off leaves
    right-click and `/` to the page;
  - name prompt on first join; not shown again after a restart of share
    and joiner (read from disk); a change under the gear updates presence;
  - settings survive a new share (same home directory).

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
  - a cursor anchored to an element that only exists for bob is not drawn
    for alice;
  - comments: alice's and bob's comments each appear only in their own
    list; delivered state reaches only the author;
  - addressed replies: `jampot send --to alice` shows `to you` for alice
    and nothing for bob; `--anchor` on bob's comment reaches only bob; a
    broadcast reaches everyone;
  - reconnect: one joiner drops and returns; the others' presence and
    cursors update, and its unsent comments go out.

## Open question

- Comment mode default: on (current behaviour) or off (opt in). Keep on
  until decided.
