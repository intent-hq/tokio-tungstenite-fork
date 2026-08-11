# INTENT-HQ FORK — Maintenance

This is [intent-hq](https://github.com/intent-hq)'s fork of
[snapview/tokio-tungstenite](https://github.com/snapview/tokio-tungstenite). This
document is the fork's maintenance policy. Everything fork-specific in the tree is
marked `INTENT-HQ FORK`.

## What the fork carries

The `deflate` branch carries, on top of upstream:

- **`deflate` feature** — forwards the
  [intent-hq/tungstenite-rs](https://github.com/intent-hq/tungstenite-rs) fork's
  permessage-deflate (RFC 7692) support (`deflate = ["tungstenite/deflate"]`).
- **tungstenite dependency redirect** — `tungstenite` is consumed as a git
  dependency on the intent-hq fork's `deflate` branch, so the whole graph resolves
  to the forks.
- **CI additions** — a deflate-feature test in CI and the `upstream-sync.yml`
  automation described below.

## Branch layout

| Branch | Purpose |
|--------|---------|
| `deflate` | **Default branch.** Upstream + the fork delta. What consumers pin. Never pushed to by automation. |
| `master` | Pristine mirror of upstream `master`. No fork commits. |
| `sync/upstream-<date>` | Integration branches created by the upstream-sync workflow. Disposable. |

`deflate` is the default branch so that scheduled workflows (which only fire from the
default branch) run against the fork's real code.

## Upstream sync automation

`.github/workflows/upstream-sync.yml` runs weekly (and via `workflow_dispatch`). It:

1. Fetches upstream `master` and the latest upstream release tag (`vX.Y.Z`).
2. Detects whether upstream has commits/releases not yet merged into `deflate`.
3. If so, attempts an automated merge of upstream into `sync/upstream-<date>` and
   reports whether it merged cleanly and whether the gates pass (fmt / clippy /
   test with deflate).

**Notification mechanism: a single draft PR** against `deflate` (opened once, then
refreshed on subsequent runs). Issues are disabled on this fork, so a draft PR is the
chosen surface — it appears in the PR list, notifies watchers, and carries the merge
and gate results in its body. On merge conflicts the integration branch points at
upstream head so GitHub surfaces the conflicts on the PR.

The workflow **never pushes to `deflate`**. A human (or agent) performs the actual
version bump. Note: the draft PR is created with the workflow's `GITHUB_TOKEN`, so
the regular CI workflow does not auto-run on it — the gate results in the PR body
and a manual CI run on the integration branch are the signal.

## Upstream version bump procedure

Bump the tungstenite fork **first** (see its MAINTENANCE.md), then:

1. `git fetch upstream --tags` (upstream = `https://github.com/snapview/tokio-tungstenite`).
2. Fast-forward `master` to upstream `master` (mirror only, no fork commits).
3. On a branch off `deflate`, `git merge vX.Y.Z` (the upstream **release tag**).
   Resolve conflicts — the usual hotspot is `Cargo.toml` (keep the git dependency
   on the tungstenite fork and bump its `version` to match).
4. Re-run the gates: `cargo +nightly fmt --all --check`,
   `cargo clippy --lib --tests --features connect,handshake,deflate -- -D warnings`,
   `cargo test --release --features connect,handshake,deflate`, and the **Autobahn**
   suite (`scripts/autobahn-client.sh` / `scripts/autobahn-server.sh` — CI runs
   these too).
5. Merge to `deflate` (PR or fast-forward) once green.
6. Retag (see below) and update consumers.

## Tagging convention

Fork releases are tagged `vX.Y.Z-deflate.N`, where `vX.Y.Z` is the upstream release
the branch is based on and `N` is a monotonically increasing fork revision on that
base (e.g. `v0.30.0-deflate.1`, `v0.30.0-deflate.2`, `v0.31.0-deflate.1`). Plain
`vX.Y.Z` tags are upstream's — never create fork tags without the `-deflate.N`
suffix.

## Consumers

[intentd](https://github.com/intent-hq/intentd) depends on this fork directly
(`tokio-tungstenite = { git = "...", branch = "deflate", features = [...] }`) and
**pins exact revisions via `Cargo.lock`** — moving `deflate` never silently changes
intentd builds; a bump here requires a `Cargo.lock` update in intentd.

## Retirement

If upstream lands permessage-deflate support, this fork is retired: consumers move to
the upstream release and the fork is archived. Tracked in
[intent-hq/monorepo#1969](https://github.com/intent-hq/monorepo/issues/1969).
