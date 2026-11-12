# jamjam

jamjam lets an agent share a site it is working on with a few people. They
join from their own machine, point at the page and comment; the comments
reach the agent's session. It is the multi-user successor of
[komtar](https://github.com/rot256/komtar), peer to peer over
[iroh](https://iroh.computer): there is no jamjam server. See
[DESIGN.md](DESIGN.md) for the design and the implementation plan; delivery
to the agent (`jamjam recv`, `jamjam send`, T3) is not built yet.

## Installation

Build and install from a checkout:

```sh
cargo install --path .
```

## Usage

Share a static directory (pages reload when files change):

```sh
jamjam serve ./public
```

Or share a running development server:

```sh
jamjam proxy http://127.0.0.1:5173
```

Both print a party token. Anyone with the token can see the site and
comment until the share stops; restart the share to revoke it. Others join
with:

```sh
jamjam join <token> [--name <name>]
```

`join` opens `http://<session>.localhost:<port>/` in the browser (Chromium
or Firefox; Safari does not resolve `*.localhost`). Then:

1. **Right-click any element** to comment on it.
2. **Select text first** to include the exact passage.
3. **Press `/`** for a page-level comment when focus is not in a field.
4. **Press Enter** to send; Shift+Enter adds a newline.

The joiner keeps its key in `~/.config/jamjam/key`; the share shows its
name next to a fingerprint of that key.
