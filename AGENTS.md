# Repository instructions

`jamparty` is the multi-user, peer-to-peer successor of komtar; docs/ui.md
specifies the joiner's overlay. Keep the runtime as one self-contained
binary: the browser client (JavaScript and CSS) is written in
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
npm run build && git diff --exit-code -- assets
uv run ruff format --check .
uv run ruff check .
uv run basedpyright
uv run pytest
cargo build --release
```

After changing what the proxy sends or injects, the joiner, the request
rules, the protocol or the browser client (the paths
`.github/workflows/frameworks.yml` watches), also run the framework tests:
real dev servers through jamparty, in both browsers (Linux; Node.js 22.23 or
later on PATH; they download Hugo and Zola):

```sh
uv run pytest -m frameworks
```

Do not add production integration hooks. Keep the agent-facing comment NDJSON
that `jamparty recv` prints (`"version":1`) stable within a release line;
change it only when explicitly requested. The internal peer protocol follows
the versioning policy below.

## Versioning policy

Peer compatibility follows the release line: `0.x` before 1.0, and the
major version from 1.0 onward. Prerelease suffixes do not change the line.
A minor bump in 0.x may break the peer protocol, agent socket or CLI;
patch releases must remain compatible within their line. Keep no shims,
capability negotiation or migration code for older releases. The NDJSON
`"version":1` is the independent comment/envelope format version.
Peers on different lines refuse one another; a joiner gets an install
hint only when it is behind the share. Compatible patches need no note.

## Branches and merging

Build each feature on its own branch. The feature decides the version:
a minor bump if it breaks compatibility (see above), a patch bump
otherwise. Bump first: the version change is the branch's first commit
(or is rebased in before the first commit that needs it), so no commit
anywhere carries a change under the old version. Merge into main with a
signed merge commit (`git merge --no-ff`) whose message describes the
feature. Every merge bumps the version, whether or not it is released.

## Releasing

Releasing is separate from merging: main already carries the version. Wait
for CI to pass on main, then push a signed tag that matches Cargo.toml:

```sh
git tag -s vX.Y.Z -m "Release vX.Y.Z" && git push origin vX.Y.Z
```

The tag runs .github/workflows/release.yml: it checks the tag against
Cargo.toml, reruns CI and framework tests, builds the binaries, publishes
the crate to crates.io, then creates the GitHub release. Publishing waits
for every build to succeed.

If publication has not occurred, retry with workflow_dispatch using the
tag as the ref; if a code fix is needed, fix it on main and move the signed
tag only after confirming that version is absent from crates.io.
Once published, keep the tag unchanged; publishing changed code requires
a new version. If crates.io publishing succeeded but the GitHub release
job failed, rerun only the failed release job in the original run.
Do not use workflow_dispatch or rerun all jobs for that case: they attempt
cargo publish again, which fails because the version already exists.
