#!/usr/bin/env bash
set -euo pipefail

CARGO_TOML="Cargo.toml"

usage() {
  cat <<'USAGE'
Usage:
  ./scripts/release.sh <version|patch|minor|major>

Examples:
  ./scripts/release.sh patch
  ./scripts/release.sh minor
  ./scripts/release.sh major
  ./scripts/release.sh 1.2.3
  ./scripts/release.sh 2.0.0-beta.1

Single command flow:
  1) run quality gate
  2) bump version locally
  3) commit release on main
  4) push main
  5) create + push tag
USAGE
}

die() {
  echo "Error: $*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "missing required command: $1"
}

current_version() {
  grep '^version' "$CARGO_TOML" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

is_semver() {
  [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z][0-9A-Za-z.+-]*)?$ ]]
}

bump_version() {
  local current="$1"
  local part="$2"
  local major minor patch
  local base="${current%%-*}"
  IFS='.' read -r major minor patch <<<"$base"

  case "$part" in
    major) echo "$((major + 1)).0.0" ;;
    minor) echo "${major}.$((minor + 1)).0" ;;
    patch) echo "${major}.${minor}.$((patch + 1))" ;;
    *)
      is_semver "$part" || die "invalid version '$part' (expected semver, patch, minor, or major)"
      echo "$part"
      ;;
  esac
}

ensure_clean_worktree() {
  [[ -z "$(git status --porcelain)" ]] || die "working directory is not clean"
}

ensure_on_main() {
  local branch
  branch="$(git branch --show-current)"
  [[ "$branch" == "main" ]] || die "release flow must run from main"
}

ensure_main_synced() {
  git fetch origin main --tags >/dev/null
  local local_head remote_head
  local_head="$(git rev-parse HEAD)"
  remote_head="$(git rev-parse origin/main)"
  [[ "$local_head" == "$remote_head" ]] || die "local main is not up to date with origin/main"
}

version_is_greater() {
  local candidate="$1"
  local current="$2"
  [[ "$candidate" != "$current" ]] || return 1

  local candidate_base="${candidate%%[-+]*}"
  local current_base="${current%%[-+]*}"
  local candidate_major candidate_minor candidate_patch
  local current_major current_minor current_patch
  IFS='.' read -r candidate_major candidate_minor candidate_patch <<<"$candidate_base"
  IFS='.' read -r current_major current_minor current_patch <<<"$current_base"

  for part in major minor patch; do
    local candidate_value current_value
    case "$part" in
      major)
        candidate_value="$candidate_major"
        current_value="$current_major"
        ;;
      minor)
        candidate_value="$candidate_minor"
        current_value="$current_minor"
        ;;
      patch)
        candidate_value="$candidate_patch"
        current_value="$current_patch"
        ;;
    esac
    if ((candidate_value > current_value)); then
      return 0
    fi
    if ((candidate_value < current_value)); then
      return 1
    fi
  done

  local candidate_is_prerelease=0
  local current_is_prerelease=0
  [[ "$candidate" == *-* ]] && candidate_is_prerelease=1
  [[ "$current" == *-* ]] && current_is_prerelease=1

  # Same base version: stable release outranks its prereleases.
  [[ "$candidate_is_prerelease" -eq 0 && "$current_is_prerelease" -eq 1 ]]
}

tag_exists() {
  local tag="$1"
  if git rev-parse "refs/tags/$tag" >/dev/null 2>&1; then
    return 0
  fi
  if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null 2>&1; then
    return 0
  fi
  return 1
}

ensure_tag_absent() {
  local tag="$1"
  if tag_exists "$tag"; then
    die "tag $tag already exists (local or remote)"
  fi
}

resolve_next_available_version() {
  local current="$1"
  local part="$2"
  local candidate attempts=0

  candidate="$(bump_version "$current" "$part")"
  while tag_exists "v${candidate}"; do
    attempts=$((attempts + 1))
    [[ $attempts -le 100 ]] || die "too many occupied tags while resolving next ${part} version"
    candidate="$(bump_version "$candidate" "$part")"
  done

  printf '%s\n' "$candidate"
}

prepare_release_metadata() {
  local requested="$1"
  local current new_version tag

  current="$(current_version)"
  case "$requested" in
    patch|minor|major)
      new_version="$(resolve_next_available_version "$current" "$requested")"
      ;;
    *)
      new_version="$(bump_version "$current" "$requested")"
      version_is_greater "$new_version" "$current" || {
        die "exact version $new_version must be greater than current version $current"
      }
      ensure_tag_absent "v${new_version}"
      ;;
  esac

  tag="v${new_version}"

  echo "$current" "$new_version" "$tag"
}

run_quality_gate() {
  echo "Running quality checks..."
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-targets --all-features
  echo "All checks passed."
}

ensure_release_tooling() {
  require_cmd cargo
  require_cmd git
  require_cmd sed
}

apply_version_and_commit() {
  local new_version="$1"
  local tag="$2"

  sed -i.bak "s/^version = \".*\"/version = \"${new_version}\"/" "$CARGO_TOML"
  rm -f "${CARGO_TOML}.bak"
  cargo check --quiet

  git add "$CARGO_TOML" Cargo.lock
  git diff --cached --quiet && die "version update produced no commit diff"
  git commit -m "chore: release ${tag}"
}

push_main_and_verify() {
  local expected_version="$1"

  git push origin main
  git fetch origin main --tags >/dev/null

  local remote_version
  remote_version="$(git show origin/main:"$CARGO_TOML" | grep '^version' | head -1 | sed 's/.*"\(.*\)".*/\1/')"
  [[ "$remote_version" == "$expected_version" ]] || {
    die "remote main version mismatch after push: expected $expected_version, found $remote_version"
  }
}

publish_tag() {
  local tag="$1"
  ensure_tag_absent "$tag"
  git tag -a "$tag" -m "Release ${tag}"
  git push origin "$tag"
  echo "Published tag $tag"
}

release_and_publish() {
  local requested="$1"
  local current new_version tag

  ensure_release_tooling
  ensure_clean_worktree
  ensure_on_main
  ensure_main_synced

  read -r current new_version tag < <(prepare_release_metadata "$requested")

  echo "Current version: $current"
  echo "New version:     $new_version"
  echo "Tag:             $tag"
  echo

  run_quality_gate
  echo

  apply_version_and_commit "$new_version" "$tag"
  push_main_and_verify "$new_version"
  publish_tag "$tag"
}

main() {
  if [[ $# -ne 1 ]]; then
    usage
    exit 1
  fi

  case "${1:-}" in
    -h|--help|help)
      usage
      exit 0
      ;;
    patch|minor|major|[0-9]*)
      release_and_publish "$1"
      ;;
    *)
      usage
      die "invalid release target '$1'"
      ;;
  esac
}

main "$@"
