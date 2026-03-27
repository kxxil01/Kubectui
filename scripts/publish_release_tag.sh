#!/usr/bin/env bash
set -euo pipefail

CARGO_TOML="Cargo.toml"

current_version() {
  grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Error: working directory is not clean." >&2
  exit 1
fi

CURRENT_BRANCH="$(git branch --show-current)"
if [[ "$CURRENT_BRANCH" != "main" ]]; then
  echo "Error: publish_release_tag.sh must run from main." >&2
  exit 1
fi

git fetch origin main --tags >/dev/null
LOCAL_HEAD="$(git rev-parse HEAD)"
REMOTE_HEAD="$(git rev-parse origin/main)"
if [[ "$LOCAL_HEAD" != "$REMOTE_HEAD" ]]; then
  echo "Error: local main is not up to date with origin/main." >&2
  exit 1
fi

VERSION="$(current_version)"
TAG="v${VERSION}"
if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Error: tag $TAG already exists." >&2
  exit 1
fi

git tag -a "$TAG" -m "Release ${TAG}"
git push origin "$TAG"

echo "Published tag $TAG"
