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
