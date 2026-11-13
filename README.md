# jampot

jampot lets an agent share a site it is working on with a few people. They
join from their own machine, point at the page and comment; the comments
reach the agent's session, and the agent's answers appear on the page for
everyone. It is the multi-user successor of
[komtar](https://github.com/rot256/komtar), peer to peer over
[iroh](https://iroh.computer): there is no jampot server. jampot was
called jamjam during design, and briefly nabe.

## Installation

Build and install from a checkout:

```sh
cargo install --path .
```

## Sharing

Share a static directory (pages reload when files change):

```sh
jampot serve ./public
```

Or share a running development server:

```sh
jampot proxy http://127.0.0.1:5173
```

Both print a party token and a share id. Anyone with the token can see the
site and comment until the share stops; restart the share to revoke it.
Comments are prompts to the agent, so hand the token only to people you
would let instruct it.

Where comments go is chosen with options on `serve` and `proxy`:

- **Nothing** (the default): the share keeps them until the agent runs
  `jampot recv`, which waits for comments, prints them as NDJSON and exits.
- **A T3 thread**: `--t3-url <mcp url> --t3-token-file <file>`, with
  `--thread <id>` or `T3CODE_THREAD_ID`. Each batch is queued in the thread
  as a new turn.
- **A command**: `-- <command> [args...]` runs once per batch with the
  comments as NDJSON on stdin; exit status 0 means delivered.
  `--command-timeout <secs>` (default 60) bounds each run.

The agent answers on the page with `jampot send [message]` (Markdown, from
stdin when no message is given), seen by everyone connected.
`--anchor <comment id>` places the answer next to that comment and sends it
only to the comment's author; `--to <name>` sends it only to that person
(their page marks it "to you"; when two people share a name, the share
lists them and `--to key:<fingerprint>` picks one), and `--to all` sends it to
everyone again. `recv` and `send` reach the running share through a
socket only the same user can use; with several shares running, pass
`--share <id>`.

Agents set themselves up from `jampot agent`, which lists guides for T3,
Claude, Codex and others; `jampot agent <kind>` prints one.
[assets/jampot/SKILL.md](assets/jampot/SKILL.md) is a skill that points
there.

## Joining

```sh
jampot join <token> [--name <name>]
```

`jampot join -` reads the token from stdin, which keeps it out of `ps` and
shell history. `join` opens `http://<session>.localhost:<port>/` in the
browser (Chromium or Firefox; Safari does not resolve `*.localhost`). Then:

On first join the page asks for your name ("Join the potluck."). Then:

1. **Right-click any element** to comment on it.
2. **Select text first** to include the exact passage.
3. **Press `/`** for a page-level comment when focus is not in a field.
4. **Press Enter** to send; Shift+Enter adds a newline.

The pill at the bottom right shows the connection (green: connected,
amber: reconnecting, grey: the share is unreachable) and how many of your
comments are queued; click it for your comments, each queued, delivered
or not sent (with "send again"). The gear next to it opens the controls:

- **Comment mode** (key `C`): off, right-click and `/` belong to the page
  again.
- **Show my cursor** (key `X`, off by default): others on the same page
  see your pointer with your name; nothing is sent while it is off.
- Click a key to rebind it; your name; who is here.

Keys work when no text field has focus. Your name, keys and switches are
kept in `~/.config/jampot/settings.json` (mode 0600), so they carry over
to the next share; `--name` overrides the name for one run.

The joiner keeps its key in `~/.config/jampot/key` (mode 0600; a key file
others can read is refused); the share shows its name next to a
fingerprint of that key. Connections are direct when possible, otherwise
through iroh's public relays, which forward only end-to-end encrypted
traffic.

## Releasing

Set `version` in `Cargo.toml`, commit, and push a tag `vX.Y.Z` matching it:

```sh
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

The release workflow (`.github/workflows/release.yml`) refuses a tag that
differs from the `Cargo.toml` version, runs the checks, builds the binaries,
publishes the crate and creates the GitHub release. Publishing to crates.io
needs either a `CARGO_REGISTRY_TOKEN` repository secret or crates.io trusted
publishing configured for `rot256/jampot` and workflow `release.yml`.
