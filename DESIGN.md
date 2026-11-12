# jamjam design

*Let's JamJam?*

jamjam lets an agent share a site it is working on with a fixed set of
people. They sign in with GitHub, open the session's link, point at the
page and comment. Their comments are pushed to the agent's session (a T3
thread, or any command), and the agent's answers appear on the page for
everyone.

jamjam starts as a copy of komtar (v0.3.1, history kept). komtar stays the
single-user, local tool; jamjam is the hosted, multi-user one.

## Goals

- One command for an agent to share a directory or a dev server, from any
  machine; the link works immediately. No Caddy, DNS or deploy step per
  share.
- Access by GitHub account (numeric user id): a global allowlist,
  optionally narrowed per session. GitHub sign-in is the only gate; session
  links are identifiers, not secrets.
- Every comment carries the commenter's GitHub identity, set by the server.
- Comments are delivered to the session's target without the agent
  polling, and delivery happens where the agent runs: the server holds no
  credentials for the agent's environment.
- Each session is its own origin (`<seed>.<domain>`), at its root: absolute
  links (`/style.css`, `/@vite/client`) keep working, and the browser keeps
  the cookies, storage and service workers of different projects apart.
- Persist only access and configuration, as JSON. Queued comments, answer
  history and live-reload state are in memory.
- A dumb server. It knows sessions, access, comments and answers, and
  nothing about agents: where comments go and how is decided by the
  client. Delivery details are an opaque client-owned blob the server
  stores and hands back. New clients (another agent runner, an editor
  plugin) work against the same server unchanged.
- Testability drives the structure (see Code structure and Testing).

## Non-goals

- No collaborative editing of the page itself; people comment, the agent
  edits.
- No database. If JSON stops being enough, revisit.
- No production traffic: shared sites are previews.
- No sandboxing of the agent. A `command` target is where a sandboxed
  agent would plug in.

## Overview

```
reviewer's browser
  -> Caddy (TLS: <domain>, *.<domain>)
  -> jamjam server ------------------- iroh (QUIC, key-authenticated) ----+
       front door: GitHub sign-in, pairing, entry                         |
       session hosts: access checks, comments, answers, proxying          |
                                                                          v
                                                   jamjam share (anywhere: t3, a laptop, CI)
                                                     serves the directory or proxies
                                                     the dev server, injects the client,
                                                     delivers comments locally (T3 or a command)
```

One binary, two roles:

- **`jamjam server`**: internet-facing behind Caddy. Signs people in,
  keeps the added share keys, decides access, routes each session host to its share
  over iroh, queues comments until the share pulls them, relays
  answers to viewers. It holds no credentials for any agent environment
  and never reads project files.
- **`jamjam share`**: run by the agent next to the project. Dials the
  server over iroh, registers a session, then serves HTTP requests that
  arrive as iroh streams: static files with live reload, or a proxy to a
  dev server on its own loopback. It injects the browser client (komtar's
  code, unchanged in spirit), pulls its sessions' comments from the
  server and delivers them locally.

The share dials out, so it works from behind NAT and needs no listening
port, mount or firewall rule. A dev server bound to `127.0.0.1` works as
is.

## Identities

| Who | Identity | Proven by |
|---|---|---|
| Reviewer | GitHub numeric user id | GitHub OAuth, then jamjam's signed cookies |
| Share | iroh node id (Ed25519 public key) | The QUIC handshake |
| Server | iroh node id | The QUIC handshake; pinned by every share |

GitHub logins can be renamed and re-registered, and compare
case-insensitively; jamjam stores and compares numeric user ids only. The
config may name people by login for convenience; the server resolves
logins to ids at startup through the GitHub API and logs the mapping. A
login that no longer resolves stops startup.

## Pairing

Each place that runs jamjam has its own persistent key pair. A person adds
that key to their account on the front door, once.

1. The server URL is built into the binary: `https://<domain>`, with the
   domain read at compile time from `domain.txt` in the repository root
   (`include_str!`, trimmed; a placeholder under `.invalid` until the
   domain is chosen, so an unconfigured build cannot reach anything). It changes only when set explicitly:
   `--server <url>` or `JAMJAM_SERVER` (tests and self-hosting use this).
   On first contact with a server the CLI fetches
   `<server>/.well-known/jamjam` over HTTPS (the server's iroh node id,
   address and relay; trusted through the server's TLS certificate) and
   pins the node id per server URL in `~/.config/jamjam/servers.json`. A
   later change of node id is refused until the pin is removed by hand
   (`jamjam forget-server <url>`). On first use any command also creates
   `~/.config/jamjam/key` (the iroh secret key, mode 0600); it persists, so
   the environment keeps its identity.
2. `jamjam code` prints this environment's pairing code: its public key
   with a checksum and the suggested label, e.g.
   `jamjam1-4f1ca90e77b2...-t3@mtrx`. It is public; printing or pasting
   it reveals nothing secret.
3. The person signs in on the front door, opens "Your devices", pastes the
   code and confirms the label. The server records the key in `keys.json`
   (`{key, user_id, label, added, last_seen}`).
4. From then on the environment connects. A key not in `keys.json` is
   refused during the handshake, with a reason the CLI shows ("this device
   is not added; run `jamjam code` and add it at https://<domain>/devices").
   The server accepts nothing from unknown keys: there is no pairing
   request, code store or approval state on the server.

Rules:

- Only people in the config's `share` list (a subset of `allow`) may add
  keys. Commenters cannot publish.
- A key acts as the person who added it. Sessions it creates are owned by
  that person and that key; a key can manage and answer only sessions it
  created.
- "Your devices" lists a person's keys (label, last seen) with a remove
  button. Removing closes the key's connection at once and marks its
  sessions offline. Removing someone from `share` disables all their keys.
- The page warns that adding a code lets that device publish sites under
  the person's name: pasting a code someone else sent you is the phishing
  risk, and the warning is the defence.
- Headless environments (CI, a remote box) print their code once and are
  added from any browser.

## Sessions

A session is one shared site.

```json
{
  "seed": "k3x9q2mfa7wd5brt",
  "label": "landing page review",
  "owner": 1234567,
  "key": "4f1ca90e77b2...",
  "created": "2026-11-12T16:00:00Z",
  "expires": "2026-11-19T16:00:00Z",
  "access": [2345678, 3456789],
  "client": { "kind": "t3", "thread": "1b3f246a-...", "title": "Landing page redesign" }
}
```

- `seed`: 16 characters of lowercase base32 (80 bits), generated by the
  server. Hostnames are lowercased by browsers, so seeds are lowercase from
  the start. It names the session and its host `<seed>.<domain>`.
- `access`: user ids allowed in this session, besides the owner. Absent
  means everyone on the global allowlist; an empty list or an id not on
  the allowlist is rejected at creation. Effective access is always within
  the allowlist.
- `expires`: optional; default and maximum from config (7 and 30 days).
  Expired sessions are removed and their hosts answer 404.
- `client`: opaque JSON owned by the client (capped at 4 KB). The server
  stores it, returns it on `resume`, `list` and `status`, and never reads
  it. The `jamjam` CLI keeps its delivery target there (see Clients), so a
  resumed share (same key, after a restart) knows where to deliver without
  being told again; another client keeps whatever it needs.
  Because the server never reads it, a client may encrypt it (for
  example with a key derived from its iroh secret key); the server stores
  ciphertext the same way. The `jamjam` CLI keeps it plaintext and free of
  secrets, so `list` and the server's logs stay useful; a client that does
  store secrets there must encrypt them.

Lifecycle:

1. The agent runs `jamjam share ./public` or
   `jamjam share http://127.0.0.1:5173` (options: `--label`,
   `--access alice,bob`, `--expires 3d`, `--target <name>`,
   `--thread <id>`).
2. The share connects, resolves and checks its delivery target (its own
   business, see Clients), and asks the server to create a session,
   passing its `client` blob. It prints the link
   `https://<domain>/s/<seed>` and whatever the client wants to say about
   its target.
3. The share serves until stopped. The session outlives it: while the
   share is not connected, the session host shows a "site offline" page and
   comments queue on the server. `jamjam share --resume <seed> ...`
   reattaches (only the key that created the session may).
4. `jamjam unshare <seed>`, expiry, or revoking the key ends the session;
   open streams to its viewers are closed.

## Origins and requests

jamjam has its own registrable domain (`<domain>`). On a subdomain of a
shared parent (say `jamjam.rot256.dev`), shared sites would be same-site
with everything else on that parent: they could set cookies for the whole
parent domain and would receive other services' SameSite cookies.

- **`<domain>`, the front door.** Sign-in, pairing, entry links, "your
  sessions" and "your devices". Never serves shared content.
- **`<seed>.<domain>`, one per session.** The shared site at the root, plus
  jamjam's reserved paths under `/_jamjam/`.

Why one origin per session: browsers isolate cookies, localStorage,
IndexedDB, caches and service workers by origin. On one shared origin a
project's `session` cookie would reach every other project's server, two
projects would overwrite each other's cookies, and a service worker from
one project would intercept every other session. Separate origins fix all
of it, and the site's own cookies pass through jamjam untouched.

`Host` is parsed strictly: exactly `<domain>`, or exactly one label of
16 base32 characters followed by `.<domain>`; no trailing dot, no port
other than the configured one, nothing else. Anything else is 404.

Front door paths:

| Path | What |
|---|---|
| `/s/<seed>` | Entry link: checks access, hands the browser to the session host |
| `/auth/login`, `/auth/callback` | GitHub sign-in |
| `/auth/logout` | POST, Origin-checked |
| `/devices` | The person's keys: add (paste a pairing code) and remove; POSTs are Origin-checked |
| `/.well-known/jamjam` | Public: the server's iroh node id, address and relay, fetched and pinned by the CLI on first contact |
| `/` | The person's sessions (only those they may open) and devices |

Session host paths:

| Path | What |
|---|---|
| `/_jamjam/enter?token=...` | Swaps a one-time entry token for this host's cookie |
| `/_jamjam/api/comments` | POST a comment (JSON only) |
| `/_jamjam/api/status` | Delivery state of the viewer's own comments (one request, on load) |
| `/_jamjam/api/messages` | Answers, delivery-state changes, share presence and the status line (SSE, `: keep-alive` every 15 s as in komtar) |
| anything else | Proxied to the share over iroh (the site, the client script, live reload) |

Entering a session:

1. `https://<domain>/s/<seed>?next=<path>`: without a front-door cookie,
   sign in first (the entry URL is kept). Not on the allowlist: 403. No
   access to this session, or no such session: 404, the same bytes either
   way.
2. Redirect to `https://<seed>.<domain>/_jamjam/enter?token=<t>`. The
   token is random, single use, valid 60 s, bound to the user id, the seed
   and a nonce cookie set on the front door just before (so a token sent to
   someone else cannot sign them in as the sender), and kept in memory.
3. The session host redeems it, sets its cookie, and redirects to `next`
   (validated: a path on this host, starting with a single `/`) or `/`.
   `/enter` sends `Referrer-Policy: no-referrer`.

A top-level GET on a session host without a valid cookie redirects to the
entry link with `next` set (for any well-formed seed, so it reveals
nothing; a loop guard stops repeats). API and SSE requests without a valid
cookie get 401, and the client shows a "sign in again" prompt.

Every request on a session host passes the authentication middleware:

1. The cookie must verify for this host and its user id must still have
   access to the session.
2. Cross-site requests are refused. Sibling session hosts are same-site, so
   `__Host-` cookies and SameSite alone do not stop session A's page from
   POSTing to, framing, or opening a WebSocket to session B with the
   reviewer's B cookie attached. So: `Sec-Fetch-Site` must be `same-origin`
   or `none`, except top-level navigations to `/_jamjam/enter`; non-GET
   requests and upgrades must carry an exact matching `Origin`; responses
   carry `Content-Security-Policy: frame-ancestors 'self'`.
3. The request goes to the share over iroh, after removing jamjam's cookie
   (from every `Cookie` header) and any `X-Jamjam-*` headers. Responses
   lose any `Set-Cookie` for jamjam's cookie names. Every other cookie
   belongs to the site and passes through both ways.

Several sessions can be open at once, in different tabs.

## Sign-in and cookies

- GitHub OAuth app, authorization code flow with PKCE and `state`, no
  scopes requested. After the callback the server reads the user id from
  `GET /user` and drops the GitHub token.
- Cookies are minted and checked in `core/`: the value is
  `(purpose, seed or "door", user id, expiry)` with an HMAC over all of it
  (key from config). A session cookie therefore fails on another session
  and as the door cookie, and the reverse.
- Front door: `__Host-jamjam-door`. Session hosts: `__Host-jamjam`. Both
  `HttpOnly`, `Secure`, path `/`, `SameSite=Lax` (Strict would loop after
  the cross-site redirect from GitHub and the entry redirect), 7 days. No
  cookie is ever set on the parent domain.
- Nothing about signed-in people is persisted. Revocation is the access
  check on every request plus closing open streams (see Revocation).
  Logout clears the door cookie; session cookies stay valid until expiry
  but fail the access check once access is removed.

## Share transport (iroh)

- The server runs an iroh endpoint with its own key (from a secret file),
  ALPN `jamjam/1`, on a public UDP port. Shares reach it directly by
  address (from their config); a self-hosted `iroh-relay` is the fallback
  for networks that block UDP. Public n0 relays and discovery are off.
- Each side pins the other: the server accepts only node ids in
  `keys.json`, refusing others in the handshake with a reason, and the
  share accepts only the server's node id.
- One connection per share process. Streams:
  - **Control** (share opens, one long-lived bidirectional stream):
    newline-delimited JSON requests and responses: create, resume, update
    and end sessions, post answers, read status, acknowledge comments.
  - **Polls** (share opens, one stream per poll): "comments for session S
    after cursor C, wait up to 60 s"; the server answers as soon as there
    are any (a new comment wakes the waiting poll at once; there is no
    periodic check anywhere), or empty at the deadline, after which the
    client asks again immediately. Latency is one hop, as with a push. Only the server answers; it never
    initiates anything on the control side.
  - **Requests** (server opens, one bidirectional stream per proxied HTTP
    request): HTTP/1.1 on the stream; the share serves it with komtar's
    hyper code. Upgrades (WebSocket, HMR) keep the stream open in both
    directions.
- Keepalive is QUIC's, not jamjam's: the share sets a keep-alive
  interval of 15 s (it is the side behind NAT; UDP mappings often expire
  after about 30 s) and both sides use a 45 s idle timeout. No
  application-level ping. When the idle timeout fires, the server marks
  the share's sessions offline at once instead of holding a dead
  connection; poll streams need nothing extra.
- Reconnection: the share reconnects with backoff and resumes its
  sessions; the server shows "site offline" meanwhile and keeps queued
  comments.
- The share forwards to dev servers with `Host` rewritten to the upstream
  and `X-Forwarded-Proto: https`; dev-server origin checks (Vite, Next)
  are covered by a browser test against a real Vite.

## Client injection

The share injects the browser client into HTML responses, as komtar does
today (it knows the live-reload revision and the upstream's encoding).
The site owns its origin, so its scripts can patch or remove the client;
jamjam does not rely on the client for anything security-relevant. What
the server guarantees is only this: every comment it accepts carries the
author and session it authenticated, and answers go only to viewers with
access. A strict CSP on the site can block the client, and HTML over 2 MB
or streamed HTML gets no client; both are documented, not worked around.

## Connection state in the page

The client shows one indicator (a dot with a tooltip in the comment UI),
driven by the answers SSE stream, without extra requests:

| State | Source |
|---|---|
| Browser to server: connected, or reconnecting | The stream's `open` and `error` events, plus a watchdog: no event or `: keep-alive` for 35 s means reconnecting |
| Share to server: agent online, or offline | `share` events: the current state when the stream opens, then on every change (the server learns of a lost share within the 45 s QUIC idle timeout) |
| The client's status line | `status` events, shown as text |

Green means connected and agent online; amber, reconnecting; grey, agent
offline or a status line is set. Comments written while it is not green
are accepted and queued as usual, and the dialog says they will be
delivered when the agent is back.

## Comments and delivery

- The client posts a comment as JSON. The server attaches `author` (user
  id and current login) and `session`, assigns an id, and queues it per
  session (cap 500, from komtar). Rate limit: 10 comments per minute per
  person (a small token bucket in `core/`, on the injected clock).
- Delivery is pulled by the client (a long poll). Each session has an
  ordered queue; every comment gets a sequence number. The client asks for
  comments after its cursor and, once it has delivered them, acknowledges
  up to a sequence number; the server drops acknowledged comments.
  Unacknowledged comments are handed out again on the next poll, so
  delivery is at least once; the `jamjam` client remembers the last
  sequence number it delivered and skips duplicates.
- Viewers learn that a comment was delivered through an event on the
  answers SSE stream, not by polling: komtar's client polls `/status`
  every 750 ms; jamjam's reads it once on load and then follows the
  stream.
- The server keeps no delivery state machine: no push, no backoff, no
  retry or hold states. A client that cannot deliver simply does not
  acknowledge (and polls again later), or stops polling. Viewers see each
  of their comments as queued or delivered (acknowledged).
- A client may set a short status line for the session (an opaque string,
  shown on the page as is, e.g. "agent unavailable: thread archived").
  The server does not interpret it.
- Comments go only to the key that owns the session.
- Restarts: queued comments are in memory and a server restart loses them.
  The client keeps each comment's text and id until `/status` reports it
  delivered; after a restart the server reports those ids unknown, and the
  client shows them as not delivered with a "send again" button.

A poll returns one JSON object per comment, with length caps:
`id`, `author`, `page`, `selector`, `selectedText`, `comment`. Text from
browsers is data inside JSON fields, never formatted into prose, so a
comment cannot forge an author line.

### Clients

The server speaks one small protocol over iroh (create, resume, end and
update sessions, poll for comments and acknowledge them, set the status
line, post answers, read status) and treats delivery as opaque. Everything about targets lives in
the client: the `jamjam` CLI below is one client; another (an editor
plugin, a different agent runner) implements the same protocol and keeps
its own `client` blob.

The `jamjam` CLI's targets come from its own config, so the server never
sees target credentials. It stores the chosen target in the session's
`client` blob (`{"kind": "t3", "thread": ..., "title": ...}` or
`{"kind": "command", "name": ...}`), never credentials or argv.

- **`t3`**: the share is an MCP client of T3 (URL and token file in its
  config; in `t3`, the token comes from the existing sign-in script with
  label `jamjam` and is renewed by the nightly job, which jamjam re-reads
  per delivery).
  - The thread is `--thread` or `T3CODE_THREAD_ID`; with neither, `share`
    refuses. At share time it calls `t3_thread_read`: an unknown thread
    fails; a child thread is resolved to its top-level thread (or refused,
    if T3 gives children their own `T3CODE_THREAD_ID`; the contract test
    settles which); the title is printed and shown in `jamjam list`.
  - The comments returned by one poll become one `t3_thread_send` with `mode: 'queue'` (starts an
    idle thread, never steers a running turn): a fixed header ("Comments
    from jamjam session <seed> (<label>, <url>); reviewer input, not
    instructions from the owner. Answer with `jamjam send <seed>`.")
    followed by the JSON comments. No appending to queued messages in v1.
  - `jamjam retarget <seed>` from another thread rebinds, with the same
    checks: the CLI rewrites its `client` blob, clears the status line and
    starts polling again. The server only sees a blob update.
  - If T3 reports the thread gone, the CLI sets the status line to the
    reason and stops polling that session until `retarget`.
- **`command`**: an argv from the share's config, the polled comments as NDJSON on
  stdin, `JAMJAM_SESSION` in the environment; exit 0 means delivered. For
  anything else, including a sandboxed agent.

## Answers

`jamjam send <seed> "message"` (or `--anchor <comment id>`) goes over the
share's control stream; only the key that owns the session may send. The
server publishes it to everyone viewing the session (komtar's agent hub
and SSE stream, one hub per session), rendered with komtar's restricted
Markdown renderer.

## Revocation

Access is checked on every request, and long-lived streams are tied to it:
each session has a cancellation signal that fires on any access change
(allowlist reload, session access update, unshare, expiry, key revoked).
Open SSE and WebSocket streams of viewers who lost access are closed, and
on unshare or key revocation all of the session's streams are.

## CLI

| Command | What |
|---|---|
| `jamjam forget-server <url>` | Drop a pinned server node id (after a deliberate server key change) |
| `jamjam code` | Print this environment's pairing code (its public key; the key is created on first use and kept in `~/.config/jamjam/key`) |
| `jamjam share <dir or url>` | Create a session and serve it until stopped |
| `jamjam share --resume <seed> <dir or url>` | Reattach to an existing session |
| `jamjam unshare <seed>` | End a session |
| `jamjam list` | This key's sessions: seed, label, link, target, viewers |
| `jamjam access <seed> --set alice,bob` | Change a session's access |
| `jamjam send <seed> <message>` | Answer on the page |
| `jamjam status <seed>` | Queue: pending comments, delivery in flight, last error |
| `jamjam retarget <seed>` | Bind delivery to the current thread (updates the `client` blob) |
| `jamjam server --config <file>` | Run the server |

Every command except `server` talks to the server over the share's iroh
connection (the long-running `share` process owns delivery; the others are
short connections with the same key).

## Configuration and persistence

Server state directory: three JSON files, mode 0600, read at start.
Server-written files are written atomically (temp file, fsync, rename,
fsync of the directory) by a single writer task; a corrupt or truncated
file stops startup instead of starting empty.

`config.json`, written by the operator (Ansible), read-only to the server
(reloaded on SIGHUP):

```json
{
  "domain": "jam.example",
  "listen_http": "127.0.0.1:3940",
  "listen_iroh": "0.0.0.0:7842",
  "relay_url": "https://relay.jam.example",
  "iroh_key_file": "/run/secrets/iroh-key",
  "cookie_key_file": "/run/secrets/cookie-key",
  "github": {
    "client_id": "...",
    "client_secret_file": "/run/secrets/github",
    "authorize_url": "https://github.com/login/oauth/authorize",
    "token_url": "https://github.com/login/oauth/access_token",
    "api_url": "https://api.github.com"
  },
  "allow": ["rot256", "alice", "bob"],
  "share": ["rot256"],
  "sessions": { "default_expiry": "7d", "max_expiry": "30d" }
}
```

Secrets are files referenced by path, never inline, so the config can
live in a repo. The GitHub URLs are configurable for tests; since pointing
them elsewhere would bypass sign-in, the server logs them at startup.

Written by the server:

- `sessions.json`: `{"version": 1, "sessions": [...]}`.
- `keys.json`: `{"version": 1, "keys": [...]}`, the paired share keys.

Share side, all optional except the key: `~/.config/jamjam/key` (iroh
secret key, created on first use), `~/.config/jamjam/servers.json` (pinned
server node ids, written by the CLI), and `~/.config/jamjam/config.json`
(delivery targets; no server address, which is built in unless overridden
by `--server` or `JAMJAM_SERVER`).

## Security model

- **Comments are prompts.** Whatever the target agent can do, a person
  with access to a session can ask it to do. For a T3 target the allowlist
  is effectively a shell allowlist for `t3`. For people you would not give
  that, use a `command` target that hands comments to a sandboxed agent.
- **The server holds no agent credentials.** It has the GitHub client
  secret, the cookie key and its iroh key. A compromised server can serve
  wrong pages, forge comments and read sessions, and hand forged comments
  to polling shares; it cannot reach T3 or any project directly. What a
  share does with comments is decided on the share's side.
- **Share keys** publish under a person's name. A stolen key can create
  sessions (possibly phishing pages) shown to that person's reviewers and
  answer in its own sessions; it cannot read or change other sessions or
  see anyone's sign-in. "Last seen" makes misuse visible; revocation is
  immediate.
- **Shared sites run JavaScript** in reviewers' browsers, each on its own
  origin, on a domain used for nothing else. A site can act as the
  reviewer within its own session (post comments as them). The
  cross-site rules stop it from acting in another session; `__Host-`
  cookies stop it from setting or overwriting jamjam's cookies.
- **Session links are identifiers, not secrets**: seeds appear in DNS
  queries, SNI and Referer headers. 80 bits and identical 404s keep them
  unguessable, but access rests on sign-in alone. One wildcard certificate
  keeps seeds out of Certificate Transparency logs.
- **The server never proxies to an address of its choosing**: requests go
  only to the share that owns the session, over its authenticated
  connection.

## Deployment (agent box)

- `jamjam` container, own podman network: HTTP published on the host's
  loopback for Caddy; the iroh UDP port published on the public address.
  No mounts of agent state, no route to `t3` (the share in `t3` dials the
  server like any other share).
- Caddy serves `<domain>` and `*.<domain>` with one wildcard certificate
  (ACME DNS-01 via deSEC: Caddy built with the deSEC DNS module, the deSEC
  token restricted to the `_acme-challenge` TXT record set), both
  reverse-proxied to the server. `iroh-relay` at `relay.<domain>`.
- DNS in `dnsconfig.js`: apex, `*` and `relay` A/AAAA to the agent box.
- Secrets from sops: GitHub client secret, cookie key, server iroh key,
  deSEC token.
- In `t3`: the `jamjam` binary, a paired key in the agent's home, a share
  config with the `t3` target, and the T3 MCP token issued by the sign-in
  script (label `jamjam`).

## Logo

`assets/logo/jamjam.svg`: two soft cookies with a raspberry jam layer,
after the Canadian Jam Jams. An original drawing, no product branding.
Used as the favicon of the front door and in the client's dialog.
Slogan: "Let's JamJam?", on the front door next to the logo.

## Code structure

Testing drives the layout: every decision that matters is a pure function
of plain values, and everything that talks to the outside world sits
behind a trait with a fake in the test suite.

| Module | Contents |
|---|---|
| `core/` | Pure, no I/O, no clock or randomness of its own: host parsing, access decisions and routes, cookies (mint and verify), entry tokens, pairing-code parsing, session model and validation, the delivery state machine, the rate limiter |
| `adapters/` | GitHub, iroh endpoint, filesystem store, T3 MCP, `command` target; each behind a trait |
| `http/` | The server's axum service (front door and session hosts) and the authentication middleware |
| `share/` | The share process: komtar's static and proxy serving and injection on hyper (not ported to axum), the control stream, local delivery |
| `server.rs` | Wiring: config, clock, rng, adapters; `Server::start(...)` returns a handle, so tests run the server in-process |

- **Clock and randomness are injected** (`Clock`, `Rng`). Tests use a
  manual clock and a seeded generator: token and code lifetimes, expiry,
  backoff and rate limits are tested by advancing the clock, never by
  sleeping.
- **No test mode.** Tests run production code paths. Hostnames are the
  only difference: browsers treat `http://*.localhost` as a secure context,
  so `__Host-` cookies work on `jam.localhost:<port>` and
  `<seed>.jam.localhost:<port>` (to be confirmed by an early spike).
  Rust and Python clients do not resolve `*.localhost`; they connect to
  127.0.0.1 with an explicit `Host`.
- **State is inspectable** (`jamjam status`, `jamjam list`): tests assert
  on state instead of timing; operators get the same view.

## Authentication middleware

Authentication is one tower layer around the whole server service, not a
check in each handler.

- **Deny by default.** The layer wraps the complete service, outside the
  router, so the fallback (the proxy) is covered too. It parses the host,
  collects the request facts (path, method, `Sec-Fetch-Site`, `Origin`,
  the verified cookie if any) and calls `core::access::decide` once.
  `decide` returns either a response (redirect, 401, 403, 404) or a
  `Route` plus an `Access { user, seed }`; dispatch uses that `Route`, so
  the path is parsed in exactly one place.
- **Public routes are an explicit list** in `decide` (front door:
  `/auth/login`, `/auth/callback`, `/.well-known/jamjam`; session host:
  `/_jamjam/enter`). A route added later is protected unless it is
  added to that list.
- **Handlers take the decision as an extractor** (`Access`, or `Entry` for
  the public ones), so a handler cannot read the user or session from
  anywhere else. The extractor fails closed (500 and a log line) if the
  layer did not run.
- **The proxy is just another handler** behind the same layer.
- **Share connections** are authenticated by the iroh handshake against
  `keys.json`; every control request is checked against the key's
  sessions in `core/` the same way.

## Testing

| Layer | What | How |
|---|---|---|
| Core unit tests | Host parsing, access matrix, cookies, tokens, pairing, delivery state machine, rate limiter, store | Pure functions, manual clock, seeded rng, `proptest` |
| In-process integration | Sign-in, pairing, routing, proxying over iroh, streaming, upgrades, revocation, restarts | `Server::start` plus shares in one process, iroh with direct local addresses and no relay, fake GitHub |
| Browser | Comments by two people, isolation between sessions, the client, live reload, HMR | pytest and Playwright against the real binaries (extends komtar's suite) |
| T3 contract | Real `t3_thread_read` / `t3_thread_send` behaviour, child threads | On the box, from the nightly update job, one throwaway thread |
| Deployment | Caddy, wildcard certificate, SSE and WebSockets through Caddy, iroh reachability | In the infra repo, after a deploy |

Fakes, kept in the test suite:

- **GitHub**: authorize, token and `/user`; the test chooses the user, or
  makes a step fail (bad `state`, reused code, `/user` error, renamed
  login).
- **T3 MCP**: records calls; idle, busy, unknown and child threads; a
  thread that disappears mid-session. Its responses are kept in step with
  the contract test.
- **Command targets**: scripts that append to a file, exit 1, or hang.
- **Upstreams**: static fixtures, a streaming server, an SSE server, a
  WebSocket echo, compressed HTML, and a real Vite dev server for HMR.

Must-have cases:

- **Host parsing**: fuzzed `Host` values, case variants, trailing dot,
  ports, extra labels.
- **Access matrix**: user (none, not allowlisted, allowlisted without
  access, owner, allowed) x host (front door, session A, session B, unknown
  seed) x route, each with its expected result in a table. Unknown and
  unauthorized seeds give byte-identical 404s. Access removed is denied on
  the next request.
- **Cookies**: A's cookie on B, a session cookie as the door cookie and the
  reverse, expired, tampered; `Set-Cookie` for jamjam names stripped from
  share responses; multiple `Cookie` headers.
- **Cross-site rules**: a page on A POSTing, framing and opening a
  WebSocket to B is refused (browser test); `Origin` checks on writes and
  upgrades.
- **Every route behind the middleware**: requests to a set of paths
  (`//_jamjam/enter`, `/%5Fjamjam/...`, case variants, trailing slashes,
  the fallback) without a cookie are all refused.
- **Entry flow**: tokens single use, 60 s, bound to user, seed and nonce;
  `next` cannot redirect off-host; deep links survive sign-in.
- **Pairing**: codes with a bad checksum or for an already added key are
  rejected; only `share` people add keys; an unknown key is refused in the
  handshake; a removed key's connection closes and its sessions go
  offline; `jamjam code` prints the same code across runs.
- **Delivery**: property test over random sequences of comments, polls,
  acknowledgements (including none, stale and out-of-range), disconnects
  and clock steps: nothing acknowledged is handed out again, nothing
  unacknowledged is lost while the server runs, the client skips
  duplicates, queue cap, rate limit; long polls return at the deadline.
  On the manual clock: a comment posted while a poll waits is returned
  without the clock advancing (no hidden timer), and one posted between
  polls is returned by the next poll at once.
- **Revocation**: a viewer removed while streaming loses the SSE and
  WebSocket streams.
- **Connection state (browser)**: the indicator turns grey when the
  share's connection is dropped (idle timeout on the manual clock in the
  in-process tests; a killed share in the browser test), green when it
  reconnects, amber when the browser's stream is cut, and shows a status
  line set by the share.
- **Restarts**: the client shows comments lost by a server restart as not
  delivered; the store reloads; a corrupt store stops startup.
- **Isolation (browser)**: cookies set by site A (server or JavaScript)
  never reach site B; localStorage is separate; a service worker
  registered in A does not control B.
- **Proxying**: site cookies pass through both ways; SSE, chunked and
  upgraded streams are not buffered (chunk timing); an offline share shows
  the offline page; resume reattaches.

## Implementation plan

1. Rename: crate, binary, reserved paths, client identifiers, docs;
   komtar's tests pass under the new name.
2. Spike (done 2026-11-12):
   - iroh 1.4.1 (healthy: 1.0 since June 2026, frequent releases), pinned.
     In-process with direct addresses and no relay: an unknown key is
     refused in the handshake with its reason; ten concurrent HTTP/1.1
     requests over per-request bi streams (hyper on both ends) run in
     parallel; an HTTP upgrade carries data both ways; a share killed with
     SIGKILL is noticed after the idle timeout (1.5 s idle: 1.65 s), and
     keepalive holds an idle connection open.
   - Browsers: in Chromium and Firefox `http://*.localhost` is a secure
     context, `__Host-` cookies set and stay per host, sibling hosts never
     see each other's cookies. A credentialed fetch from A to B carries B's
     cookies with `Sec-Fetch-Site: same-site` (confirming the cross-site
     rules are needed, and that the header is there to enforce them).
     WebKit does not resolve `*.localhost`: browser tests run in Chromium
     and Firefox.
3. `core/` with its unit and property tests, before any I/O.
4. Share over iroh: control stream, request streams on komtar's hyper
   serving, injection, reconnection; in-process tests with a stub server.
5. Server: config, store, `Server::start`, the middleware, host routing,
   proxy over iroh; fake GitHub and integration tests.
6. Sign-in and pairing end to end; browser tests on `*.localhost`,
   including isolation and cross-site rules.
7. Comments, long-poll delivery, `command` target, answers, revocation.
8. `t3` target: fake MCP, then the contract test on the box.
9. Deployment in the infra repo (domain, Caddy with deSEC, relay, `t3`
   setup), with its deployment tests.

Each step keeps every test layer it touches green.

## Crates

iroh (transport), axum and tower (server and middleware), hyper and
hyper-util (share side, from komtar), tokio, reqwest (GitHub API, T3 MCP;
the OAuth flow is two calls, no `oauth2` crate), hmac and sha2 (cookies),
serde/serde_json, proptest for tests; komtar's existing dependencies for
static serving, file watching and Markdown.

## Open questions

- Which registrable domain?
- iroh: version to pin, and whether `iroh-relay` runs behind Caddy or on
  its own port.
- Do T3 child threads get their own `T3CODE_THREAD_ID`? (Contract test.)
- Should answers reach commenters any other way (GitHub notification,
  email), or is the page enough?
