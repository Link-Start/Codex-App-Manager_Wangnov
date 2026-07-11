#!/usr/bin/env bash
# Fail closed unless an exact, already-validated release commit is reachable
# from the repository's live default branch. The fetch intentionally allows the
# default branch to move forward after the tag was created; equality with HEAD
# would reject valid older release commits.
set -euo pipefail

repo="${1:-release-source}"
release_source_sha="${2:-}"
default_branch="${3:-}"

[[ "$release_source_sha" =~ ^[0-9a-f]{40}$ ]] || {
  echo "::error::release source must be a lowercase 40-character commit SHA" >&2
  exit 1
}
[[ -n "$default_branch" ]] || {
  echo "::error::default branch is missing" >&2
  exit 1
}
git -C "$repo" check-ref-format --branch "$default_branch" >/dev/null 2>&1 || {
  echo "::error::invalid default branch name" >&2
  exit 1
}

remote_ref="refs/remotes/origin/$default_branch"
git -C "$repo" fetch --no-tags origin "+refs/heads/$default_branch:$remote_ref"
git -C "$repo" cat-file -e "$release_source_sha^{commit}" 2>/dev/null || {
  echo "::error::release source commit is missing after fetching the default branch" >&2
  exit 1
}

if git -C "$repo" merge-base --is-ancestor "$release_source_sha" "$remote_ref"; then
  echo "Release source $release_source_sha is merged into origin/$default_branch"
else
  status=$?
  if [[ "$status" -eq 1 ]]; then
    echo "::error::release source $release_source_sha is not an ancestor of origin/$default_branch" >&2
  else
    echo "::error::could not verify release source ancestry (git merge-base exit $status)" >&2
  fi
  exit 1
fi
