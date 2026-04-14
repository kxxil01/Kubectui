#!/usr/bin/env bash
set -euo pipefail

CARGO_TOML="Cargo.toml"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/release.sh <version|patch|minor|major>

Examples:
  ./scripts/release.sh patch
  ./scripts/release.sh minor
  ./scripts/release.sh 1.2.3
  ./scripts/release.sh 2.0.0-beta.1

Single command flow:
  1) run quality gate
  2) bump version on release branch
  3) open release PR
  4) merge release PR immediately
  5) fast-forward local main
  6) create + push release tag
EOF
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

remote_branch_exists() {
  local branch="$1"
  git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1
}

cleanup_stale_release_branch() {
  local branch="$1"
  local current_branch
  current_branch="$(git branch --show-current)"

  if git show-ref --verify --quiet "refs/heads/$branch"; then
    if git merge-base --is-ancestor "$branch" origin/main; then
      if [[ "$current_branch" == "$branch" ]]; then
        git checkout main >/dev/null
      fi
      git branch -D "$branch" >/dev/null
      echo "Deleted stale local branch $branch"
    else
      die "local branch $branch already exists and is not merged into main"
    fi
  fi

  if remote_branch_exists "$branch"; then
    git fetch origin "$branch" --depth=1 >/dev/null
    if git merge-base --is-ancestor FETCH_HEAD origin/main; then
      git push origin --delete "$branch" >/dev/null
      echo "Deleted stale remote branch $branch"
    else
      die "remote branch $branch already exists and is not merged into main"
    fi
  fi
}

ensure_release_branch_absent() {
  local branch="$1"
  cleanup_stale_release_branch "$branch"

  if git show-ref --verify --quiet "refs/heads/$branch"; then
    die "local branch $branch already exists"
  fi
  if remote_branch_exists "$branch"; then
    die "remote branch $branch already exists"
  fi
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
  require_cmd gh
  require_cmd sed
  gh auth status >/dev/null 2>&1 || die "gh is not authenticated. Run 'gh auth login' first."
}

release_pr_body() {
  local tag="$1"
  local new_version="$2"
  printf '## Why\n- prepare the version bump for %s\n- keep release preparation on the normal branch and PR path\n\n## How\n- update Cargo.toml and Cargo.lock to %s\n- run the standard quality gate before creating the release PR\n\n## Tests\n- `cargo fmt --all -- --check`\n- `cargo clippy --all-targets --all-features -- -D warnings`\n- `cargo test --all-targets --all-features`\n' "$tag" "$new_version"
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
  local current new_version tag release_branch

  current="$(current_version)"
  case "$requested" in
    patch|minor|major)
      new_version="$(resolve_next_available_version "$current" "$requested")"
      ;;
    *)
      new_version="$(bump_version "$current" "$requested")"
      ensure_tag_absent "v${new_version}"
      ;;
  esac

  tag="v${new_version}"
  release_branch="chore/release-${tag}"

  ensure_release_branch_absent "$release_branch"

  echo "$current" "$new_version" "$tag" "$release_branch"
}

create_release_pr() {
  local new_version="$1"
  local tag="$2"
  local release_branch="$3"
  local pr_url

  sed -i.bak "s/^version = \".*\"/version = \"${new_version}\"/" "$CARGO_TOML"
  rm -f "${CARGO_TOML}.bak"
  cargo check --quiet

  echo "Creating release branch and PR..." >&2
  git checkout -b "$release_branch"
  git add "$CARGO_TOML" Cargo.lock
  git commit -m "chore: release ${tag}"
  git push -u origin "$release_branch"
  pr_url="$(gh pr create \
    --base main \
    --head "$release_branch" \
    --title "chore: release ${tag}" \
    --body "$(release_pr_body "$tag" "$new_version")")"

  printf '%s\n' "$pr_url"
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
  local current new_version tag release_branch pr_url

  ensure_release_tooling
  ensure_clean_worktree
  ensure_on_main
  ensure_main_synced

  read -r current new_version tag release_branch < <(prepare_release_metadata "$requested")

  echo "Current version: $current"
  echo "New version:     $new_version"
  echo "Tag:             $tag"
  echo "Release branch:  $release_branch"
  echo

  run_quality_gate
  echo

  pr_url="$(create_release_pr "$new_version" "$tag" "$release_branch")"

  echo
  echo "Merging $pr_url ..."
  gh pr merge "$pr_url" --squash --delete-branch

  git checkout main
  git pull --ff-only origin main
  local merged_version
  merged_version="$(current_version)"
  [[ "$merged_version" == "$new_version" ]] || die "post-merge version mismatch: expected $new_version on main, found $merged_version"
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
