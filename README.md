# jamjam

jamjam is the hosted, multi-user successor of
[komtar](https://github.com/rot256/komtar), under construction: an agent
shares the site it is working on, reviewers sign in with GitHub and comment
on the page, and the comments reach the agent's session. See
[DESIGN.md](DESIGN.md) for the plan. Today it still behaves like komtar: a
local, single-user tool, documented below.

## Installation

Build and install from a checkout:

```sh
cargo install --path .
```

## Usage

Serve a static site with live reload:

```sh
jamjam serve ./public
open http://127.0.0.1:3939
```

jamjam serves files and directory `index.html` files, returning a normal 404
when a path does not exist. When a served file changes, open pages reload. If
the suggestion dialog is open, jamjam waits until the suggestion is submitted
or dismissed so the draft is not lost. Dotfiles (including the configured
FIFO) are not exposed by the static server.

For a project that already has a development server, proxy it through jamjam:

```sh
jamjam proxy http://127.0.0.1:5173
open http://127.0.0.1:3939
```

The original shorthand remains supported:

```sh
jamjam http://127.0.0.1:5173
```

Ask your coding agent to wait for queued suggestions:

```sh
jamjam recv
```

`recv` waits for a nonempty delivery, prints one batch of unchanged v1 NDJSON
records, and exits. After processing that batch, the agent runs `jamjam recv`
again directly. Run `jamjam agent` to print the complete workflow and a sample
feedback record.

1. **Right-Click Any Element:** Open the edit suggestion box.
2. **Select Text First:** Include the exact passage as context.
3. **Right-Click an Agent Answer:** Comment on that response and let the next
   requested answer attach to it.
4. **Press `/`:** Open an unanchored page-level comment box when focus is not
   in an editable field.
5. **Press Enter to Submit:** Use Shift+Enter to add a newline.

## Collaborative Editing

Keep jamjam on loopback and expose it only to your tailnet:

```sh
jamjam serve ./public
```

In another terminal:

```sh
tailscale serve 3939
```

Share the URL printed by Tailscale Serve. Everyone in your tailnet can point at
the page and suggest changes, and all suggestions are queued in the same `.jamjam`
FIFO for your coding agent. jamjam does not provide its own authentication.
