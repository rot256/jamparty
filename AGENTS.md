# Repository instructions

`nabe` is the hosted, multi-user successor of komtar; DESIGN.md describes
the target and its implementation plan. Keep the runtime as one
self-contained binary: JavaScript and CSS belong in `assets/client.js` and are
embedded with `include_str!`.

Run the checks below before handing off changes:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
npm ci
npm run lint
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest
cargo build --release
```

Do not add production integration hooks. Preserve the version-1
newline-delimited JSON schema unless a breaking format change is explicitly
requested.
