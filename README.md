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
jamparty proxy http://127.0.0.1:5173        # a running dev server
jamparty serve ./public -- ./on-comments.sh # hand comments to a command
jamparty serve ./public -- t3 thread send "$T3CODE_THREAD_ID" -
```

Anyone with the printed token can see the site and comment until the
share stops.

Commands receive each comment batch as NDJSON on stdin, in the same format
as `jamparty recv`. Inside T3, the example queues comments as a new turn;
start the share in the background. A subagent should pass the top-level
thread's id. Failed commands retry with backoff, so skip duplicate comment
ids. See [command delivery](docs/delivery.md) or `jamparty agent t3`.

## Join

```sh
jamparty join <token>
jamparty join -          # read the token from stdin
jamparty join <token> --port 8080  # serve on this port (default: the dev server's)
```

## Answer (agent)

```sh
jamparty agent                       # guides for agents
jamparty recv                        # wait for comments, print them as NDJSON
jamparty send "Fixed, thanks."       # to everyone
jamparty send --anchor <id> "Done."  # next to a comment, to its author
jamparty send --to alice "…"         # to one person (or --to key:<fingerprint>)
```
