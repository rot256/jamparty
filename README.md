# jampot

Share a site you are working on; people comment on it from their own
browser and the comments reach your agent. Peer to peer over
[iroh](https://iroh.computer).

## Install

```sh
cargo install --locked --git https://github.com/rot256/jampot jampot
```

## Share

```sh
jampot serve ./public                     # a static directory
jampot proxy http://127.0.0.1:5173        # a running dev server
jampot serve ./public -- ./on-comments.sh # hand comments to a command
jampot serve ./public --t3-url <url> --t3-token-file <file> --thread <id>
```

Anyone with the printed token can see the site and comment until the
share stops.

## Join

```sh
jampot join <token>
jampot join -          # read the token from stdin
jampot join <token> --port 8080  # serve on this port (default: the dev server's)
```

## Answer (agent)

```sh
jampot agent                       # guides for agents
jampot recv                        # wait for comments, print them as NDJSON
jampot send "Fixed, thanks."       # to everyone
jampot send --anchor <id> "Done."  # next to a comment, to its author
jampot send --to alice "…"         # to one person (or --to key:<fingerprint>)
```

## Release

```sh
git tag -a v0.1.0 -m "Release v0.1.0" && git push origin v0.1.0
```
