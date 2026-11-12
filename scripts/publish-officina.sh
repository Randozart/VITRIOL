#!/usr/bin/env bash
# publish-officina.sh — release the officina subtree as a standalone
# snapshot (plan: .opencode/plans/officina-publishing-2026-11-11.md).
#
# VITRIOL is canonical; the public officina repo receives exact subtree
# snapshots (I2). Nothing here hand-edits the published tree — the split
# branch is a pure `git subtree split` of one monorepo commit.
#
# Pipeline: clean-tree check → gates (npm/tsc/canaries/cargo) → secrets
# canary → subtree split → tag officina-v<version> → push branch+tag.
# Prints the published provenance (tag, split commit, source commit).
#
# Usage: scripts/publish-officina.sh [--remote URL] [--dry-run]
# Env:   OFFICINA_PUBLISH_REMOTE (default: officina — add it once via
#        `git remote add officina https://github.com/Randozart/officina.git`)

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
SUBTREE="officina"
BRANCH="publish/officina"
REMOTE="${OFFICINA_PUBLISH_REMOTE:-officina}"
DRY_RUN=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1 ;;
    --remote) REMOTE="${2:?--remote needs a URL}"; shift ;;
    *) fail "unknown flag: $1" ;;
  esac
  shift
done

fail() { echo "publish-officina: $*" >&2; exit 1; }

# ── 1. Clean tree, main-push awareness ──────────────────────────────────
[[ -z "$(git status --porcelain)" ]] || fail "working tree dirty — commit or stash first"
git fetch --quiet origin 2>/dev/null || true
LOCAL_HEAD="$(git rev-parse HEAD)"
if git rev-parse --verify origin/main >/dev/null 2>&1; then
  if [[ "$(git rev-parse origin/main)" != "$LOCAL_HEAD" ]]; then
    AHEAD="$(git rev-list origin/main..HEAD --count)"
    # Warn, don't block: the published tag carries its own provenance
    # (split commit + source hash printed below). The operator's push
    # cadence on the private origin is their own business.
    echo "WARN: main is ${AHEAD} commit(s) ahead of origin/main"
  fi
else
  echo "WARN: no origin/main — provenance relies on the printed source hash"
fi
echo "== source commit: $LOCAL_HEAD"

# ── 2. Gates (same as every officina rework) ────────────────────────────
echo "== gates: npm test"
(cd officina && npm test --silent >/tmp/opencode/publish-npm.log 2>&1) || fail "npm test failed — see /tmp/opencode/publish-npm.log"
echo "== gates: tsc"
(cd officina && npx tsc --noEmit >/tmp/opencode/publish-tsc.log 2>&1) || fail "tsc failed — see /tmp/opencode/publish-tsc.log"
echo "== gates: build-patch canaries"
(cd officina && node runtime/build-patch.mjs >/tmp/opencode/publish-patch.log 2>&1) || fail "canaries failed — see /tmp/opencode/publish-patch.log"
echo "== gates: cargo test (tui)"
(cd officina/tui && cargo test --quiet >/tmp/opencode/publish-cargo.log 2>&1) || fail "cargo test failed — see /tmp/opencode/publish-cargo.log"

# ── 3. Secrets canary (I3: split carries full path history) ─────────────
echo "== secrets canary (history of ${SUBTREE}/)"
# Whole-history scan of the subtree paths for key/env/pem files.
if git log --format= --name-only -- "${SUBTREE}" | sort -u | grep -E '\.(env|key|pem)$' >/dev/null 2>&1; then
  fail "key/env/pem files exist in ${SUBTREE} history — remove before publishing"
fi
# Key-shaped strings, working tree AND full path history. The allowlist
# carries KNOWN test fixtures (obvious dummies, e.g. the alphabet key in
# the sensitive-prompt routing test); anything else fails the publish.
ALLOW_FAKE='sk-abcdefghijklmnopqrstuvwxyz0123456789'
PATTERN='sk-[A-Za-z0-9_-]{20,}|gho_[A-Za-z0-9]{20,}|ghp_[A-Za-z0-9]{20,}'
LEAKS="$(git grep -nE "$PATTERN" -- "${SUBTREE}" | grep -vF "$ALLOW_FAKE" || true)"
[[ -z "$LEAKS" ]] || { echo "$LEAKS" | head -5 >&2; fail "secret-shaped strings in ${SUBTREE} working tree"; }
HIST_LEAKS="$(git log -p --format= -- "${SUBTREE}" | grep -nE "$PATTERN" | grep -vF "$ALLOW_FAKE" | head -5 || true)"
[[ -z "$HIST_LEAKS" ]] || { echo "$HIST_LEAKS" >&2; fail "secret-shaped strings in ${SUBTREE} history"; }

# ── 4. Version + split + tag ────────────────────────────────────────────
VERSION="$(node -p "require('./officina/package.json').version")"
TAG="officina-v${VERSION}"
if git rev-parse "$TAG" >/dev/null 2>&1; then
  fail "tag ${TAG} exists — bump officina/package.json version"
fi

echo "== subtree split"
# Fresh deterministic split of the current commit every run.
git branch -D "${BRANCH}" >/dev/null 2>&1 || true
SPLIT="$(git subtree split -P "${SUBTREE}")"
echo "== split commit: $SPLIT (tree of ${LOCAL_HEAD})"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "== DRY RUN: would tag ${TAG} at ${SPLIT} and push to remote '${REMOTE}'"
  exit 0
fi

git tag "$TAG" "$SPLIT"
git push "$REMOTE" "refs/heads/${BRANCH}:refs/heads/main" "refs/tags/${TAG}"

# ── 5. Provenance (fingerprint discipline) ──────────────────────────────
cat <<EOF
PUBLISHED-OFFICINA: tag=${TAG} split=${SPLIT} source=${LOCAL_HEAD} remote=${REMOTE}
gates: npm+tsc+canaries+cargo OK · secrets canary OK
EOF
