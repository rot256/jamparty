# Ideas

**Ideas only: none of this is planned or decided.** Brainstorming kept for
later; nothing here binds the design (DESIGN.md, docs/ui.md). Each entry
says what it is, why it might be worth it, and what is unclear.

## Identity through SSH and GitHub keys

*Idea, 2026-11-13.*

Let the sharer say "only john may join" and have jampot prove who john is,
using the SSH keys john already has on GitHub, without accounts or OAuth.

Flow:

1. The joiner connects to the share as usual (iroh) and asks to
   authenticate as `john`.
2. The share runs an SSH server on an iroh stream; the joiner's jampot
   connects to it with the user's normal SSH client and agent, as
   `john@<share>` (for example through `ssh -o ProxyCommand="jampot ssh-proxy
   <token>" john@share`, or jampot speaking SSH itself).
3. The share fetches `https://github.com/john.keys` (john's SSH
   authentication keys, public) and accepts john only if he authenticates
   with one of them.
4. Over that SSH connection john's jampot sends its iroh public key, then
   closes it. The share now associates that iroh key with `john` for the
   rest of the session (and could remember it).

Why it is attractive:

- Uses keys people already have, including keys on hardware tokens or
  held by an SSH agent: the private key never has to be exported or reused
  as an iroh key, only used to sign the SSH handshake.
- Nothing to publish beyond what GitHub already shows; no jampot-specific
  GitHub setup.
- The sharer can name people by GitHub login ("john, alice") and jampot
  shows verified names instead of self-chosen ones.

Open points:

- A simpler variant without an SSH server: the share sends a challenge that
  includes the joiner's iroh key; john signs it with `ssh-keygen -Y sign`
  (SSH signatures, using the agent) and returns the signature; the share
  verifies it against `github.com/john.keys`. Same trust, far less code.
- Binding: the iroh key must be bound inside the authenticated exchange
  (signed, or sent on the SSH channel) so a man in the middle cannot swap
  it.
- GitHub is trusted at authentication time; a key removed from GitHub
  later stays associated until the share restarts (or re-checks).
- Cache `github.com/<login>.keys` on the publisher's disk (for example
  `~/.cache/jampot/github-keys/<login>`, with the fetch time): fewer
  requests to GitHub, no rate-limit trouble, and authentication keeps
  working while GitHub is unreachable. Refresh after a TTL (a day?), and
  re-fetch once when someone presents a key the cache doesn't have (they
  may have just added it). The cost: a key removed from GitHub is still
  accepted until the TTL runs out; a manual refresh (or a short TTL) covers
  the "key leaked" case.
- Logins can be renamed; storing GitHub's numeric user id (from the API)
  alongside the login avoids a re-registered name inheriting access.
- How this combines with the party token: token to reach the share, SSH or
  signature to get a verified name (and perhaps per-person access).

## Cursors across screen sizes

*Idea, 2026-11-13. Refinements to the cursor design in docs/ui.md.*

- Element missing in the receiver's layout (desktop-only sidebar, mobile
  menu): fall back to the nearest existing ancestor and draw the cursor
  faintly there, instead of not at all.
- Reflowing text: anchor to the smallest element under the pointer, and
  over text to the character (`caretPositionFromPoint` / caret range,
  sending a text offset), so the cursor points at the same word at any
  width.
- Element off-screen for the receiver: a small edge marker in the person's
  colour pointing up or down; clicking it scrolls there.
- Fragile selectors (`nth-child` chains differ when the DOM differs by
  size): prefer ids and stable attributes, and send a short text sample of
  the element so the receiver can check it found the same one. Comment
  anchoring would benefit too.
- Tests would run two joiners at different window sizes (desktop and
  phone width).

## Also kept for later (from DESIGN.md "Later")

- An MCP endpoint on the running share, so agents get `send` and friends
  as tools.
- A web gateway peer that joins on behalf of browsers, so people without
  jampot installed can take part.
