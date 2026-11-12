# jamjam design

*Let's JamJam?*

jamjam lets an agent share a site it is working on with a few people.
They join from their own machine, point at the page and comment. Their
comments go straight to the agent's session (a T3 thread, or any
command), and the agent's answers appear on the page for everyone.

jamjam starts as a copy of komtar (v0.3.1, history kept). komtar is the
single-user, local tool; jamjam connects several people to one share,
peer to peer over iroh. There is no jamjam server.

## Goals

- `jamjam` in a directory shares it (a static directory or a dev server)
  and prints a ticket; `jamjam join <ticket>` opens it in the joiner's
  browser. Nothing to deploy, no accounts, no OAuth.
- Access is held by the running share and nowhere else: who may join, by
  key, lives in the publisher's process and ends with it. People are added from the command line by GitHub account, by raw
  key, or with an invite code.
- Every comment carries the identity of the key that sent it, as the
  sharer recorded it.
- Comments reach the agent without it polling; answers reach every
  joiner live.
- Each session is its own browser origin on the joiner's machine, so
  sites' cookies, storage and service workers stay apart.
- Testability drives the structure (see Code structure and Testing).
- Other ends can be added later without changing the core: a web
  gateway (see Future: web gateway) is just another peer.

## Non-goals

- No hosted service, web front door, OAuth or public domain in this
  version.
- No store-and-forward: comments flow while the share runs (when it is
  down there is no site to comment on either).
- No collaborative editing of the page itself; people comment, the agent
  edits.

## Overview

```
joiner's machine                                         sharer's machine
browser
  -> http://<sid>.localhost:<port>  (jamjam join)  ==iroh==>  jamjam (share)
       local HTTP server, per session                          serves the directory or proxies
       forwards requests, comments                             the dev server, injects the client,
       receives answers, reload, presence                      checks keys against its list, 
                                                               delivers comments (T3 or a command)
```

One binary, two roles:

- **Host** (`jamjam` / `jamjam share` in a directory): an iroh endpoint
  that accepts joiners whose keys are on its access list (or who redeem
  an invite), serves HTTP requests arriving as iroh streams (komtar's
  static and proxy code, with the browser client injected), receives
  comments, delivers them locally and broadcasts answers.
- **Joiner** (`jamjam join <ticket>`): connects to the host, runs a local
  HTTP server on `127.0.0.1` for the joiner's browser, forwards each
  request over iroh, and serves jamjam's own endpoints (comments, answers,
  reload, presence) locally from its connection.

Connectivity is iroh's: direct when possible (hole punching), through a
relay otherwise. The relay only forwards end-to-end encrypted QUIC.

## Identities and keys

- Each environment (machine, container, CI runner) has one persistent
  Ed25519 key in `~/.config/jamjam/key` (iroh secret key, mode 0600,
  created on first use). It is the joiner identity.
- `jamjam code` prints the environment's code: its public key with a
  checksum and a suggested label, `jamjam1-<64 hex>-<8 hex>-<label>`
  (checksum: first 4 bytes of SHA-256 over key and label; labels 1–40
  characters of `[A-Za-z0-9._@+-]`). It is public; printing or pasting it
  reveals nothing secret.
- `jamjam code --ssh` prints the same public key as an OpenSSH line,
  `ssh-ed25519 AAAA... jamjam@<label>`, for publishing on GitHub (below).
- A host process uses a fresh iroh key per run. Joiners learn it from the
  ticket and pin it: a joiner only talks to the host its ticket names.

## Adding people

Three ways that put a key on the running share's access list, and one
that does not (party):

1. **By GitHub account.** People publish their jamjam keys on GitHub as
   SSH *signing* keys (Settings, SSH keys, type "Signing"; the line from
   `jamjam code --ssh`, title starting with `jamjam`). Signing keys grant
   no access to anything on GitHub, unlike authentication keys (which is
   why `github.com/<user>.keys`, the authentication keys, is not used:
   publishing a jamjam key there would make it a push credential).
   `jamjam add alice` reads the public, unauthenticated
   `api.github.com/users/alice` (numeric id) and
   `api.github.com/users/alice/ssh_signing_keys`, keeps the Ed25519 keys
   whose title starts with `jamjam`, and stores them with the login and
   id. `jamjam refresh` re-reads every GitHub-backed entry: new keys are
   added, keys gone from GitHub are removed. Trust comes from GitHub over
   TLS at `add` and `refresh` time; joining never contacts GitHub.
2. **By raw key.** `jamjam add jamjam1-...` (someone's `jamjam code`).
   For people without GitHub, or devices kept off it.
3. **By invite.** `jamjam invite` (on a running share) prints a single-use
   invite code: the ticket plus a random 128-bit secret, valid for 1 hour
   by default (`--ttl`). `jamjam join <invite>` connects with the
   joiner's own key and redeems it; the host then adds that key to
   the access list (label from the joiner's code, marked as invited) and the
   invite is spent. Invites are held by the running host (hashed, in
   memory); a host restart voids them. The CLI warns that an invite
   grants what `add` grants (see Security model).
4. **Party.** `jamjam party` (on a running share) prints a multi-use
   code: the ticket plus a random 128-bit secret. Anyone holding it joins
   with their own key, without being added to the access list; they appear and
   comment under the label from their code, marked as party guests. The
   party lasts 1 hour by default (`--ttl`) and ends at once with
   `jamjam party --end`, when it expires, or when the share stops; ending
   it disconnects its guests. Because every comment is a prompt to the
   agent, `jamjam party` with a `t3` target requires `--yes` (or an
   interactive confirmation) and says what it grants; a `command` target
   pointing at a sandboxed agent is the intended companion.

## Access state

The access list lives only in the running share process (the publisher).
There is no access file: `jamjam add`, `remove`, `refresh`, `invite` and
`party` talk to the running share over its runtime socket, and the
list, invites and party end when the share stops (as its ticket does).

Entries:

- **GitHub**: login, numeric id, and the keys found (with their titles).
- **Key**: one key and the label the sharer gave it (default: the label
  in its code).
- **Invited**: a key added by redeeming an invite, with its label and when.

Comments are attributed to the GitHub login or the label, never to
anything the joiner sends. Removing an entry disconnects its keys at once;
adding one lets it join immediately. Access can be given at start too:
`jamjam share --add alice --add jamjam1-...` (resolved before the ticket
is printed).

## CLI

| Command | What |
|---|---|
| `jamjam` (or `jamjam share [dir or url] [--add ...] [--thread <id> or --command <name>]`) | Share this directory (or proxy a dev server URL); print the ticket; hold the access list; run until stopped |
| `jamjam join <ticket or invite>` | Connect, start the local server, open the browser, run until stopped |
| `jamjam code [--ssh]` | Print this environment's code (or its key as an SSH line) |
| `jamjam add <github login or code>` | Add a GitHub account's jamjam keys, or one raw key, to the running share |
| `jamjam remove <login, label or key prefix>` | Remove an entry from the running share |
| `jamjam refresh` | Re-read the running share's GitHub-backed entries |
| `jamjam invite [--ttl 1h]` | Print a single-use invite for the running share |
| `jamjam party [--ttl 1h] [--yes] / --end` | Print a multi-use code anyone can join with, or end the party |
| `jamjam status` | Who has access, who is connected, the ticket, delivery state |
| `jamjam send <message> [--anchor <comment id>]` | Answer on the page (to the share running for this directory) |
| `jamjam retarget` | Bind delivery to the current T3 thread |

Every command except `share`, `join` and `code` reaches the running host through
a Unix socket in the user's runtime directory (`$XDG_RUNTIME_DIR/jamjam/`
or `~/.cache/jamjam/run/`), one per shared directory, mode 0600: only the
user's own processes can drive their shares.

## Tickets

`jamjam share` prints a ticket: `jamjamt1-<base32>` encoding the host's
iroh node id, its relay URL and direct addresses, and a session id (80
random bits, new per run). Invite and party codes are tickets with a
secret appended (marked single-use or multi-use).
A ticket is not a secret: holding it lets you try to connect, and the
host refuses keys not on its access list. A new run gives a new ticket; links
need not be stable.

## Transport

- ALPN `jamjam/1`. The host accepts a connection only if the joiner's key
  is on the access list, or if the joiner's first control message presents a
  valid invite or party secret (nothing else is accepted from an unlisted
  key, and the connection closes on anything else). Refusals carry a reason the joiner
  shows ("your key is not on this share; ask the sharer to `jamjam add`
  you or for an invite"). The joiner accepts only the node id in its
  ticket.
- Streams, all opened by the joiner:
  - **Control** (one, long-lived, newline-delimited JSON): joiner sends
    comments (and an invite redemption first, if any); host sends
    acknowledgements, answers, delivery-state changes, live-reload
    revisions, the status line and presence (who is connected). Messages
    carry a version and size limits.
  - **Requests** (one bidirectional stream per HTTP request): HTTP/1.1 on
    the stream; the host serves it with komtar's hyper code. Upgrades
    (WebSocket, HMR) keep the stream open both ways.
- Keepalive is QUIC's: 15 s keep-alive interval, 45 s idle timeout. A lost
  joiner disappears from presence within the idle timeout; a lost host is
  shown in the joiner's page.
- Reconnection: the joiner reconnects with backoff to the same host
  (same ticket); a restarted host has a new ticket.
- Relays: iroh's default public relays to start with; `JAMJAM_RELAY`
  (or the config) selects a self-hosted `iroh-relay` instead.

## The joiner's local server

The browser talks to `http://<sid>.localhost:<port>/`: `<sid>` is the
session id from the ticket, the port is chosen at random and bound on
`127.0.0.1` only.

- One hostname per session: browsers separate cookies by host, not by
  port, so `<sid>.localhost` keeps different sessions' cookies, storage
  and service workers apart. `*.localhost` resolves to loopback and is a
  secure context in Chromium and Firefox (spike, 2026-11-12); WebKit does
  not resolve it.
- `Host` must equal exactly `<sid>.localhost:<port>` (defeats DNS
  rebinding: another site resolving its name to 127.0.0.1 still sends its
  own name).
- Cross-site rules, since any website the joiner visits can send requests
  to localhost: top-level document navigations (GET/HEAD with
  `Sec-Fetch-Mode: navigate`, `Sec-Fetch-Dest: document`) are allowed
  from anywhere; every other request needs `Sec-Fetch-Site: same-origin`
  (or none, for non-browser clients), and non-GET requests and upgrades
  need an exact `Origin` match; responses carry
  `Content-Security-Policy: frame-ancestors 'self'`.
- Reserved paths under `/_jamjam/` are answered by the joiner itself:
  `api/comments` (POST, JSON), `api/status` (once on load),
  `api/events` (SSE: answers, delivery state, presence, status line,
  reload; `: keep-alive` every 15 s). Everything else goes to the host.
- No cookies of jamjam's own: the joiner process is the joiner's
  identity, so nothing in the browser needs to authenticate.

## Client injection

The host injects komtar's browser client into HTML responses (it knows
the live-reload revision and the upstream's encoding). The site owns its
origin, so its scripts can patch or remove the client; nothing
security-relevant depends on it. Authorship is set by the host from the
authenticated key. A strict CSP on the site can block the client, and
HTML over 2 MB or streamed HTML gets no client; both are documented.

## Comments and delivery

- The joiner posts a comment over the control stream. The host attaches
  `author` (the GitHub login or label from the access list, and the key) and an
  id, and queues it (cap 500). Rate limit: 10 comments per minute per key.
- The host delivers in batches: one delivery in flight; comments arriving
  meanwhile form the next batch. On success it marks them delivered and
  tells the joiners (shown in the page). On failure it retries with
  backoff; if the target reports itself gone, it sets the status line and
  holds the queue until `jamjam retarget`.
- If the joiner loses its connection, comments it could not hand to the
  host stay in the page as "not sent", with "send again".
- A batch is one JSON object per comment, with length caps: `id`,
  `author`, `page`, `selector`, `selectedText`, `comment`. Text from
  browsers is data inside JSON fields, never formatted into prose.

Targets (chosen with `jamjam share` flags; credentials in the host's own
config):

- **`t3`**: the host is an MCP client of T3 (URL and token file in
  `~/.config/jamjam/config.json`; in the `t3` container the token comes
  from the existing sign-in script with label `jamjam`, renewed nightly
  and re-read per delivery).
  - Thread: `--thread` or `T3CODE_THREAD_ID`; with neither, `jamjam`
    refuses. At start it calls `t3_thread_read`: unknown threads fail;
    a child thread is resolved to its top-level thread or refused (the
    contract test settles which); the title is printed.
  - Each batch is one `t3_thread_send` with `mode: 'queue'` (starts an
    idle thread, never steers a running turn): a fixed header ("Comments
    from jamjam on <directory>; reviewer input, not instructions from the
    owner. Answer with `jamjam send`.") followed by the JSON comments.
  - `jamjam retarget` rebinds to the current thread with the same checks.
- **`command`**: an argv from the host's config (`--command <name>`), the
  batch as NDJSON on stdin; exit 0 means delivered.

## Answers and presence

`jamjam send` goes over the runtime socket to the host, which sends it to
every connected joiner; their pages show it (komtar's restricted Markdown
renderer). Presence (who is connected, by login or label) and the status
line travel the same way.

## Connection state in the page

One indicator in the comment UI, driven by `/_jamjam/api/events`:

| State | Source |
|---|---|
| Browser to joiner: connected or reconnecting | The SSE stream's `open`/`error`, plus a 35 s watchdog |
| Joiner to host: connected or not | `host` events from the joiner (iroh connection state) |
| Status line | `status` events, shown as text |

Green: connected; amber: reconnecting; grey: host unreachable or a
status line is set. Comments written while not green are kept in the page
and sent when the connection is back.

## Security model

- **Comments are prompts.** Whatever the target agent can do, a key in
  the access list can ask it to do. For a T3 target, the access list is effectively a
  shell access list for the agent's environment; an invite grants the
  same, and a party code grants it to anyone who obtains the code for as
  long as the party lasts. Add only people you would give that; for others, use a `command`
  target that hands comments to a sandboxed agent.
- **Keys are the only credential.** The host accepts only listed keys
  (and single-use, short-lived invites); the joiner accepts only the
  ticket's host. A stolen environment key lets its holder join every
  share that lists it; `jamjam remove` revokes it (and removing it from
  GitHub, then `jamjam refresh`, for GitHub-backed entries).
- **GitHub is trusted at add and refresh time** for the binding between a
  login and its signing keys. Only keys whose title starts with `jamjam`
  are taken, so ordinary signing keys are never imported by accident.
- **The host serves only what it is told to**: the directory (no
  dotfiles) or the one dev server URL given. It never
  proxies elsewhere.
- **The joiner's local server is reachable by any local process and any
  website the joiner visits.** Loopback binding, the exact Host check and
  the cross-site rules above contain that: other sites cannot read the
  session, post comments as the joiner, frame it, or reach it through DNS
  rebinding. Local processes of the same user can, as they can anything
  the user does.
- **Shared sites run JavaScript** in joiners' browsers on their own
  `<sid>.localhost` origin; they can act as the joiner within their own
  session (post comments), nothing more.
- **Relays** see who connects to whom and traffic volume, not content.

## Future: web gateway

A web end is another peer: a gateway joins sessions with its own key and
serves browsers, handling sign-in itself. For that, the protocol gains an
"on behalf of" author (the gateway asserts which person wrote a
comment), and a share's access list can hold a gateway key together with the people
it may speak for. Nothing in the host or joiner changes otherwise. The
earlier hosted design (front door, per-session origins on a dedicated
registrable domain, entry tokens, cross-site rules) is in git history
(`afe7e32`) as a starting point.

## Logo

`assets/logo/jamjam.svg`: two soft cookies with a raspberry jam layer,
after the Canadian Jam Jams. An original drawing, no product branding.
Used in the comment UI and the joiner's start page. Slogan: "Let's
JamJam?".

## Code structure

| Module | Contents |
|---|---|
| `core/` | Pure, no I/O, no clock or randomness of its own: codes, SSH key lines and tickets (format, parse), invites and parties, the access list and its checks, the joiner's request decision (Host, fetch metadata, Origin, reserved paths), the comment queue and delivery state machine, the rate limiter, ids, clock and rng traits |
| `protocol/` | Control-stream messages and limits |
| `host/` | The share: iroh accept loop, request streams on komtar's hyper serving and injection, control stream, access list, delivery, runtime socket |
| `join/` | The joiner: iroh connection, local HTTP server (axum), event stream, reconnection |
| `adapters/` | iroh endpoint setup, GitHub keys, T3 MCP client, command target, filesystem |

- Clock and randomness are injected: lifetimes, backoff and rate limits
  are tested by advancing a manual clock, never by sleeping.
- External endpoints (GitHub API base URL, T3 MCP URL) are configurable,
  so tests use fakes.
- No test mode: tests run production code paths with direct addresses and
  relays off.
- State is inspectable (`jamjam status`) for tests and people alike.

## Testing

| Layer | What | How |
|---|---|---|
| Core unit and property tests | Codes, SSH lines, tickets, invites, access list, request decisions, queue, delivery, rate limits | Pure functions, manual clock, seeded rng, `proptest` |
| In-process integration | Host and joiners over iroh: access, refusal, invites, removal while connected, requests, upgrades, comments, answers, presence, reconnection; `add`/`refresh` against a fake GitHub | Several endpoints in one test process, direct addresses, no relay |
| Browser | Comments by two joiners, isolation between sessions, cross-site rules, the client, live reload, HMR (real Vite) | pytest and Playwright (Chromium, Firefox) against the real binaries |
| GitHub contract | Real `users/<login>` and `ssh_signing_keys` responses for a test account | Opt-in, unauthenticated, against api.github.com |
| T3 contract | Real `t3_thread_read` / `t3_thread_send` | On the box, from the nightly job, one throwaway thread |

Must-have cases:

- A key not on the access list is refused with its reason; `jamjam remove`
  disconnects it; `jamjam add` lets it join without a restart; the list is
  gone after the share stops.
- Invites: single use, expire, void after a host restart; a wrong secret
  is refused without revealing whether the invite exists; an unlisted key
  can do nothing but redeem.
- Party: multi-use until it expires or is ended; ending it disconnects its
  guests; guests are never put on the access list; a `t3` target refuses to
  start a party without confirmation.
- `add` by GitHub takes only `jamjam`-titled Ed25519 signing keys,
  records login and id; `refresh` adds and removes keys; a renamed login
  is matched by id.
- A joiner refuses a host whose node id differs from the ticket.
- Joiner local server: wrong `Host` (including DNS-rebinding names) is
  refused; a page on another origin cannot POST a comment, open the event
  stream, open an upgrade, or frame the session; top-level navigation
  from another site works.
- Isolation: two sessions joined in one browser keep cookies and storage
  apart.
- Authorship: a comment's author comes from the access list for the sending
  key, whatever the joiner sends.
- Delivery: property test over random sequences of comments, delivery
  results (ok, error, gone) and clock steps: no loss while running, one
  batch in flight, backoff, hold and retarget, queue cap, rate limit.
- Proxying: SSE, chunked and upgraded streams are not buffered; a dev
  server's HMR works through join.
- Runtime socket: owned by the user, mode 0600; another user cannot use
  it.

## Implementation plan

1. Rename (done, `8661ab4`).
2. Spike (done 2026-11-12): iroh 1.4.1 in-process with direct addresses
   (key refusal, concurrent request streams, upgrades, keepalive, dead
   peer detection); per-host cookies and secure context on
   `http://*.localhost` in Chromium and Firefox, not WebKit.
3. `core/` (done for the hosted design, `d6320c1`, `3737383`): keep the
   ids, clock, rng, queue, rate limiter, pairing-code and protocol
   framing; replace the hosted parts (cookies, entry tokens, allowlist,
   persisted sessions, front-door access) with the access list, SSH
   key lines, tickets, invites and the joiner's request decision.
4. Host: iroh accept loop with key checks and invites, request streams on
   komtar's serving, control stream, access list, runtime socket.
5. Joiner: connection, local server with its request decision, event
   stream, reconnection; in-process host and joiner tests.
6. Comments, delivery (command target first), answers, presence,
   connection state; `add`/`refresh` with a fake GitHub; browser tests.
7. T3 target: fake MCP, then the contract test on the box.
8. Install jamjam in the `t3` container (infra repo) with its config and
   T3 token; end-to-end check: share from `t3`, join from the laptop,
   comment, see it in the thread.

## Crates

iroh 1.4.1 (transport), hyper and hyper-util (host serving, from komtar),
axum (joiner's local server), tokio, reqwest (GitHub API, T3 MCP),
serde/serde_json, sha2, data-encoding, getrandom, subtle; proptest for
tests; komtar's existing dependencies for static serving, file watching
and Markdown.

## Open questions

- Relays: iroh's public relays, or a self-hosted `iroh-relay` (e.g. on the
  agent box) from the start?
- Do T3 child threads get their own `T3CODE_THREAD_ID`? (Contract test.)
