#!/usr/bin/env bash
set -euo pipefail

# Release helper: bumps version, commits, tags, and pushes.
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
    IFS='.' read -r major minor patch <<< "$base"

    case "$part" in
        major) echo "$((major + 1)).0.0" ;;
        minor) echo "${major}.$((minor + 1)).0" ;;
        patch) echo "${major}.${minor}.$((patch + 1))" ;;
        *)     echo "$part" ;;  # Explicit version
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

if git rev-parse "$TAG" >/dev/null 2>&1; then
    echo "Error: tag $TAG already exists."
    exit 1
fi

echo "Current version: $CURRENT"
echo "New version:     $NEW_VERSION"
echo "Tag:             $TAG"
echo ""

# --- Quality gate ---

echo "Running quality checks..."
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
echo "All checks passed."
echo ""

# --- Bump and tag ---

# Update Cargo.toml version
sed -i.bak "s/^version = \".*\"/version = \"${NEW_VERSION}\"/" "$CARGO_TOML"
rm -f "${CARGO_TOML}.bak"

# Update Cargo.lock
cargo check --quiet

echo "Committing and tagging..."
git add "$CARGO_TOML" Cargo.lock
git commit -m "chore: release ${TAG}"
git tag -a "$TAG" -m "Release ${TAG}"

echo ""
echo "Done! To publish the release:"
echo "  git push origin main --tags"
echo ""
echo "This will trigger the release workflow to build binaries and create a GitHub Release."
