#!/usr/bin/env bash
set -euo pipefail

# Release prep helper: bumps version, creates a release branch/commit, pushes it,
# and opens a PR. Tag publication is handled separately by publish_release_tag.sh
# after the release PR merges.
#
# Usage:
#   ./scripts/release.sh <version>      # e.g. 1.0.0, 1.1.0, 2.0.0-beta.1
#   ./scripts/release.sh patch          # auto-bump patch: 1.0.0 -> 1.0.1
#   ./scripts/release.sh minor          # auto-bump minor: 1.0.0 -> 1.1.0
#   ./scripts/release.sh major          # auto-bump major: 1.0.0 -> 2.0.0

CARGO_TOML="Cargo.toml"

current_version() {
    grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

bump_version() {
    local current="$1" part="$2"
    local major minor patch
    # Strip any pre-release suffix for bumping
    local base="${current%%-*}"
    IFS='.' read -r major minor patch <<<"$base"

    case "$part" in
    major) echo "$((major + 1)).0.0" ;;
    minor) echo "${major}.$((minor + 1)).0" ;;
    patch) echo "${major}.${minor}.$((patch + 1))" ;;
    *) echo "$part" ;; # Explicit version
    esac
}

# --- Preflight checks ---

if [ $# -ne 1 ]; then
    echo "Usage: $0 <version|patch|minor|major>"
    echo ""
    echo "Examples:"
    echo "  $0 patch          # 1.0.0 -> 1.0.1"
    echo "  $0 minor          # 1.0.0 -> 1.1.0"
    echo "  $0 major          # 1.0.0 -> 2.0.0"
    echo "  $0 1.2.3          # explicit version"
    echo "  $0 2.0.0-beta.1   # pre-release"
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "Error: working directory is not clean. Commit or stash changes first."
    exit 1
fi

CURRENT=$(current_version)
NEW_VERSION=$(bump_version "$CURRENT" "$1")
TAG="v${NEW_VERSION}"
RELEASE_BRANCH="chore/release-${TAG}"

if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Error: tag $TAG already exists."
    exit 1
fi

CURRENT_BRANCH="$(git branch --show-current)"
if [ "$CURRENT_BRANCH" != "main" ]; then
    echo "Error: release prep must start from main."
    exit 1
fi

git fetch origin main >/dev/null
LOCAL_HEAD="$(git rev-parse HEAD)"
REMOTE_HEAD="$(git rev-parse origin/main)"
if [ "$LOCAL_HEAD" != "$REMOTE_HEAD" ]; then
    echo "Error: local main is not up to date with origin/main."
    exit 1
fi

echo "Current version: $CURRENT"
echo "New version:     $NEW_VERSION"
echo "Tag:             $TAG"
echo "Release branch:  $RELEASE_BRANCH"
echo ""

# --- Quality gate ---

echo "Running quality checks..."
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
echo "All checks passed."
echo ""

# --- Bump and open release PR ---

# Update Cargo.toml version
sed -i.bak "s/^version = \".*\"/version = \"${NEW_VERSION}\"/" "$CARGO_TOML"
rm -f "${CARGO_TOML}.bak"

# Update Cargo.lock
cargo check --quiet

echo "Creating release branch and commit..."
git checkout -b "$RELEASE_BRANCH"
git add "$CARGO_TOML" Cargo.lock
git commit -m "chore: release ${TAG}"
git push -u origin "$RELEASE_BRANCH"
gh pr create \
  --base main \
  --head "$RELEASE_BRANCH" \
  --title "chore: release ${TAG}" \
  --body $'## Why\n- prepare the version bump for '"${TAG}"$'\n- keep release preparation on the normal branch and PR path\n\n## How\n- update Cargo.toml and Cargo.lock to '"${NEW_VERSION}"$'\n- run the standard quality gate before creating the release PR\n\n## Tests\n- `cargo fmt --all -- --check`\n- `cargo clippy --all-targets --all-features -- -D warnings`\n- `cargo test --all-targets --all-features`'

echo ""
echo "Release PR created."
echo ""
echo "After it merges, run ./scripts/publish_release_tag.sh from clean updated main."
