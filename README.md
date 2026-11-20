# jamparty

Share a site you are working on; people comment on it from their own
browser and the comments reach your agent. Peer to peer over
[iroh](https://iroh.computer).

## Install

```sh
cargo install --locked jamparty
```

## Share

```sh
jamparty serve ./public                     # a static directory
jamparty serve build/paper.pdf              # a single file
jamparty proxy http://127.0.0.1:5173        # a running dev server
jamparty serve ./public -- ./on-comments.sh # nudge when nobody is listening
jamparty serve ./public -- t3 thread send "$T3CODE_THREAD_ID" -
```

Anyone with the printed token can see the site and comment until the
share stops. Serving a directory exposes its non-dot files and lists each
directory that has no servable `index.html`. Listings show up to 1,000
entries and skip names that aren't valid UTF-8. To share a single file,
choose a regular file with a non-dot, UTF-8 name. The start URL redirects
to that file.

Comments wait for `jamparty recv`, which prints them as NDJSON and confirms
delivery. An optional command after `--` receives only a short plain-text
reminder when comments wait and no recv listens: one nudge per backlog.
Inside T3, start the share in the background; when the nudge arrives as a new
turn, run `jamparty recv`, then answer. A subagent should pass the top-level
thread's id. Failed nudges retry with backoff without affecting the queue.
See [comment delivery](docs/delivery.md) or `jamparty agent t3`.

## Join

```sh
jamparty join <token>
jamparty join -          # read the token from stdin
jamparty join <token> --port 8080  # serve on this port (default: the dev server's)
```

## Review a PDF

Open a shared `.pdf` URL in the joiner's browser. Select text and
right-click it, or use the pen button to drag a box around a figure or
equation. The comment reaches `jamparty recv` with its page number and
coordinates. See [PDF anchors](docs/delivery.md#pdf-anchors).

For LaTeX, run these in separate terminals:

```sh
latexmk -pdf -pvc -view=none -synctex=1 -outdir=build paper.tex
jamparty serve build/paper.pdf
```

The joiner's start URL opens the PDF viewer. Rebuilds keep the reviewer's
reading position and zoom. Multi-file projects work too. Latexmk tracks
`\input` and `\include` files, bibliography, and images. With the matching
SyncTeX output, `synctex edit` reports the included source file and line.
Share the PDF alone to keep the build's `.log`, `.fls`, and `.synctex.gz`
outputs private. These can contain absolute local paths.

## Answer (agent)

```sh
jamparty agent                       # guides for agents
jamparty recv                        # wait for comments, print them as NDJSON
jamparty fetch <sha3:id> -o ./file    # fetch an attachment into your working directory
jamparty send "Fixed, thanks."       # to everyone
jamparty send --anchor <id> "Done."  # next to a comment, to its author
jamparty send --at '#intro' --page / "Start here."
jamparty send --at-pdf 'paper.pdf#page=2&box=72,140,540,180' "See this paragraph."
jamparty send --choice "Keep it" --choice "Make it bigger" "Which heading size?"
jamparty send --to alice "…"         # to one person (or --to key:<fingerprint>)
```

Use one anchor per answer. `--at` requires a share URL path such as `/` or
`/pricing.html` in `--page`. `--at-pdf` takes a PDF path relative to the
share root and a 1-based page and box in PDF points. Both forms send to
everyone by default and accept `--to`. Earlier answers appear quietly in
the comment list. **Show** takes the reader to the anchored place.

For LaTeX, compile with SyncTeX enabled (for example, `latexmk -pdf -synctex=1`),
then run `synctex view -i 6:1:paper.tex -o paper.pdf` against the matching build.
Use `Page` as the page number and convert the printed box to
`box=h,v-H,h+W,v`. The CLI reports top-left-origin big points (1/72 inch).
`h` is the left edge, `v` the bottom edge including depth, and `W` and `H`
are the full width and height. The separate `x` and `y` give a point in the
output. For example, `Page:2`, `h:133.768356`, `v:158.522720`,
`W:343.711060`, `H:8.855677` give
`'paper.pdf#page=2&box=133.77,149.67,477.48,158.52'`.

Comments can include up to four files of up to 8 MiB each. In the comment
dialog, use the paperclip, paste a file, or drop files. A comment with a
file can omit text.

`recv` reports attached files as `{"type":"file","file":"sha3:…","name":"report.pdf"}`
entries in `attachments`. Give the value of `file` to `jamparty fetch`.
Use `-o -` for stdout. Fetch checks the SHA3-256 digest and creates a new
file when given a path. Treat the bytes and displayed name as untrusted.
See [comment delivery](docs/delivery.md) for storage and eviction.

Questions offer 2–6 distinct single-line choices of at most 80 characters.
The first pick wins and removes the question for everyone; the picker sees a
brief confirmation. It arrives through the usual comment delivery with
`selector: "jamparty-agent:<answer id>"` and `comment` and optional `choice`
containing the chosen label. Typed replies do not resolve questions.

Peer compatibility follows the release line: `0.x` before 1.0, and the
major version thereafter (including prereleases). Patch releases stay
compatible; different lines refuse to connect. If the joiner is behind,
it prints `cargo install --locked jamparty@X` for the share’s version;
if it is ahead, it asks the person sharing to update.
