#!/usr/bin/env bash
set -euo pipefail

CARGO_TOML="Cargo.toml"

usage() {
  cat <<'EOF'
Usage:
  ./scripts/release.sh <version|patch|minor|major>
  ./scripts/release.sh publish
  ./scripts/release.sh ship <version|patch|minor|major>

Examples:
  ./scripts/release.sh patch
  ./scripts/release.sh minor
  ./scripts/release.sh 1.2.3
  ./scripts/release.sh 2.0.0-beta.1
  ./scripts/release.sh publish
  ./scripts/release.sh ship patch
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
    *) echo "$part" ;;
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

ensure_tag_absent() {
  local tag="$1"
  if git rev-parse "$tag" >/dev/null 2>&1; then
    die "tag $tag already exists"
  fi
}

ensure_release_branch_absent() {
  local branch="$1"
  if git show-ref --verify --quiet "refs/heads/$branch"; then
    die "local branch $branch already exists"
  fi
  if git ls-remote --exit-code --heads origin "$branch" >/dev/null 2>&1; then
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

prepare_release_metadata() {
  local requested="$1"
  local current new_version tag release_branch

  current="$(current_version)"
  new_version="$(bump_version "$current" "$requested")"
  tag="v${new_version}"
  release_branch="chore/release-${tag}"

  ensure_tag_absent "$tag"
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

prepare_release() {
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
  echo "Release PR created."
  echo "$pr_url"
  echo "After it merges, run ./scripts/release.sh publish from clean updated main."
}

publish_release() {
  local version tag

  require_cmd git
  ensure_clean_worktree
  ensure_on_main
  ensure_main_synced

  version="$(current_version)"
  tag="v${version}"

  ensure_tag_absent "$tag"

  git tag -a "$tag" -m "Release ${tag}"
  git push origin "$tag"

  echo "Published tag $tag"
}

wait_for_pr_merge() {
  local pr_ref="$1"
  local state=""

  while true; do
    state="$(gh pr view "$pr_ref" --json state --jq .state)"
    case "$state" in
      MERGED)
        return 0
        ;;
      CLOSED)
        die "release PR was closed before merge"
        ;;
    esac
    sleep 5
  done
}

ship_release() {
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
  echo "Enabling merge for $pr_url ..."
  gh pr merge "$pr_url" --squash --delete-branch --auto
  echo "Waiting for release PR to merge..."
  wait_for_pr_merge "$pr_url"

  git checkout main
  git pull --ff-only origin main
  publish_release
}

main() {
  if [[ $# -eq 0 ]]; then
    usage
    exit 0
  fi

  case "${1:-}" in
    publish)
      [[ $# -eq 1 ]] || {
        usage
        exit 1
      }
      publish_release
      ;;
    ship)
      [[ $# -eq 2 ]] || {
        usage
        exit 1
      }
      case "$2" in
        patch|minor|major|[0-9]*)
          ship_release "$2"
          ;;
        *)
          usage
          exit 1
          ;;
      esac
      ;;
    patch|minor|major|[0-9]*)
      [[ $# -eq 1 ]] || {
        usage
        exit 1
      }
      prepare_release "$1"
      ;;
    -h|--help|help)
      [[ $# -eq 1 ]] || {
        usage
        exit 1
      }
      usage
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

main "$@"
