# Repository instructions

`jampot` is the multi-user, peer-to-peer successor of komtar; DESIGN.md
describes the target and its implementation plan. Keep the runtime as one
self-contained binary: the browser client (JavaScript and CSS) is written in
TypeScript under `client/src` and bundled by `npm run build` into
`assets/client.js`, which is committed and embedded with `include_str!`.

Run the checks below before handing off changes:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo deny check
npm ci
npm run lint
npm run typecheck
npm run build && git diff --exit-code -- assets/client.js
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest
cargo build --release
```

Do not add production integration hooks. Preserve the version-1
newline-delimited JSON schema unless a breaking format change is explicitly
requested.
