#!/bin/sh
# Prints the commits of a release as Markdown, grouped by Conventional Commits type, for the
# top of the GitHub release notes. Breaking changes come first; merge commits are left out.
#
#   scripts/release-notes.sh v0.3.0            # commits since the release tag before v0.3.0
#   scripts/release-notes.sh v0.3.0 v0.2.0     # commits in v0.2.0..v0.3.0
#
# Underneath comes GitHub's own list of pull requests, new contributors and the compare link,
# under "Pull Request Summary" instead of GitHub's "What's Changed". The release workflow uses
# the output as the whole body of the release.
#
# Each line names its author as a GitHub login ("by @login"), looked up with `gh` once per
# author. Without `gh`, or when a lookup fails, the author's name from git is used instead and
# the pull request part is left out.
set -eu

tag="${1:?usage: release-notes.sh <tag> [previous tag]}"
previous="${2:-$(git describe --tags --abbrev=0 --match 'v[0-9]*' "${tag}^" 2>/dev/null || true)}"

if [ -n "$previous" ]; then
  range="${previous}..${tag}"
else
  range="$tag"
fi

# One line per author: email, then the GitHub login of one of their commits (empty if unknown).
logins="$(mktemp)"
trap 'rm -f "$logins"' EXIT
git log --no-merges --format='%ae%x09%H' "$range" | awk -F '\t' '!seen[$1]++' |
  while IFS="$(printf '\t')" read -r email commit; do
    login="$(gh api "repos/{owner}/{repo}/commits/${commit}" --jq '.author.login // empty' 2>/dev/null || true)"
    printf '%s\t%s\n' "$email" "$login"
  done > "$logins"

git log --no-merges --reverse --format='%h%x09%s%x09%ae%x09%an' "$range" | awk -F '\t' '
  NR == FNR { login[$1] = $2; next }
  BEGIN {
    count = split("breaking feat fix perf refactor docs test build ci style chore revert other", order, " ")
    title["breaking"] = "Breaking changes"
    title["feat"] = "Features"
    title["fix"] = "Fixes"
    title["perf"] = "Performance"
    title["refactor"] = "Refactoring"
    title["docs"] = "Documentation"
    title["test"] = "Tests"
    title["build"] = "Build"
    title["ci"] = "CI"
    title["style"] = "Style"
    title["chore"] = "Chores"
    title["revert"] = "Reverts"
    title["other"] = "Other"
  }
  {
    group = "other"
    if (match($2, /^[a-z]+(\([^)]*\))?!?: /)) {
      prefix = substr($2, 1, RLENGTH - 2)
      type = prefix
      sub(/[(!].*$/, "", type)
      if (prefix ~ /!$/) group = "breaking"
      else if (type in title) group = type
    }
    author = (login[$3] != "") ? "@" login[$3] : $4
    lines[group] = lines[group] "* " $2 " by " author " in " $1 "\n"
  }
  END {
    for (i = 1; i <= count; i++) {
      group = order[i]
      if (lines[group] != "") printf "## %s\n%s\n", title[group], lines[group]
    }
  }
' "$logins" -

# GitHub's generated notes for the same range. This only asks for the text; it creates nothing.
if [ -n "$previous" ]; then
  set -- -f "previous_tag_name=${previous}"
else
  set --
fi
gh api "repos/{owner}/{repo}/releases/generate-notes" -f "tag_name=${tag}" "$@" --jq '.body' 2>/dev/null |
  sed "s/^## What's Changed\$/## Pull Request Summary/" || true
