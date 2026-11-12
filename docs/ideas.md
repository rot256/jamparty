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

- A simpler variant without an SSH server (preferred if this is built): an
  SSH signature (OpenSSH "SSHSIG", what `ssh-keygen -Y sign` makes).
  - The share sends a challenge: its iroh key, the joiner's iroh key, a
    random nonce and a timestamp. john's jampot signs it in the SSHSIG
    namespace `jampot` (a signature in that namespace is useless as a git
    commit or file signature, and vice versa) and returns it; the share
    verifies it against john's keys and binds the joiner's iroh key to john.
  - Same trust as the SSH login, far less code, and the binding of the
    iroh key is inside the signed data.
  - All in the binary: verification with RustCrypto's `ssh-key` crate
    (parses `.keys` lines; verifies SSHSIG for Ed25519, ECDSA, RSA and the
    `sk-` hardware variants); signing through the user's ssh-agent over
    `SSH_AUTH_SOCK` (small protocol; works for hardware keys, the agent
    prompts for the touch); an unencrypted key file can be signed with
    directly. Gaps: a passphrase-protected key not in an agent needs a
    passphrase prompt; a hardware key not in any agent would need libfido2,
    so fall back to running `ssh-keygen -Y sign` or ask for `ssh-add`.
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
- Logins can be renamed and re-registered: store GitHub's numeric user id
  (`api.github.com/users/<login>`, field `id`) with the login and the cached
  keys, and match on the id. If a login later resolves to a different id,
  treat it as a different person (drop the cache and any association) and
  say so.
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
