#!/usr/bin/env bash
# INTENT-HQ FORK: opens or updates the single draft notification PR for the
# upstream-sync workflow. Expects env: GH_TOKEN, SYNC_BRANCH, UPSTREAM_SHA,
# LATEST_TAG, COMMITS_BEHIND, TAG_MERGED, MERGE_CLEAN, GATE_FMT, GATE_CLIPPY,
# GATE_TEST, FORK_BRANCH, UPSTREAM_REPO, UPSTREAM_BRANCH.
set -euo pipefail

TITLE="chore: upstream sync — new activity on ${UPSTREAM_REPO} ($(date -u +%Y-%m-%d))"

if [ "$MERGE_CLEAN" = "true" ]; then
  MERGE_LINE="**Merge: clean.** This branch is \`${FORK_BRANCH}\` + a merge of \`upstream/${UPSTREAM_BRANCH}\`."
  GATES_BLOCK=$(cat <<EOF
| Gate | Result |
|------|--------|
| \`cargo +nightly fmt --all --check\` | ${GATE_FMT} |
| \`cargo clippy\` (\`-D warnings\`) | ${GATE_CLIPPY} |
| \`cargo test --release\` (deflate included) | ${GATE_TEST} |
EOF
)
else
  MERGE_LINE="**Merge: CONFLICTS.** Automated merge of \`upstream/${UPSTREAM_BRANCH}\` into \`${FORK_BRANCH}\` failed; this branch points at upstream so GitHub surfaces the conflicts. Resolve manually per MAINTENANCE.md."
  GATES_BLOCK="Gates were not run (no clean merge to test)."
fi

BODY_FILE=$(mktemp)
cat > "$BODY_FILE" <<EOF
<!-- upstream-sync-bot -->
**INTENT-HQ FORK automation** — upstream \`${UPSTREAM_REPO}\` has activity not yet
merged into \`${FORK_BRANCH}\`. This draft PR is the notification (Issues are disabled
on this fork). It is refreshed on every scheduled run until the sync lands.

- Upstream \`${UPSTREAM_BRANCH}\` head: \`${UPSTREAM_SHA}\` — **${COMMITS_BEHIND} commit(s)** not in \`${FORK_BRANCH}\`
- Latest upstream release tag: \`${LATEST_TAG}\` — merged into \`${FORK_BRANCH}\`: **${TAG_MERGED}**

${MERGE_LINE}

${GATES_BLOCK}

**Do not squash-merge this PR blindly.** Follow the upstream version bump procedure in
[MAINTENANCE.md](https://github.com/${GITHUB_REPOSITORY}/blob/${FORK_BRANCH}/MAINTENANCE.md)
(merge the upstream tag, re-run gates + Autobahn, retag). The sync workflow never
pushes to \`${FORK_BRANCH}\`.
EOF

EXISTING=$(gh pr list --state open --base "$FORK_BRANCH" \
  --json number,headRefName \
  --jq "[.[] | select(.headRefName == \"$SYNC_BRANCH\")][0].number // empty")

if [ -n "$EXISTING" ]; then
  gh pr edit "$EXISTING" --title "$TITLE" --body-file "$BODY_FILE"
  echo "Updated existing sync PR #$EXISTING"
  echo "pr=updated" >> "$GITHUB_OUTPUT"
elif gh pr create --draft --base "$FORK_BRANCH" --head "$SYNC_BRANCH" \
    --title "$TITLE" --body-file "$BODY_FILE"; then
  echo "Opened new draft sync PR"
  echo "pr=created" >> "$GITHUB_OUTPUT"
else
  # Org policy may block Actions-created PRs. The workflow falls back to
  # failing visibly (see the "Fail visibly" step) with the details in the
  # run summary and the pushed sync branch.
  echo "::warning::Could not create the notification draft PR (likely org policy). Falling back to a visibly failing run."
  echo "pr=blocked" >> "$GITHUB_OUTPUT"
fi
