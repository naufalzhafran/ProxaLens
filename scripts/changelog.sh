#!/usr/bin/env bash
# Usage: scripts/changelog.sh <tag>
# Prints release notes for <tag>: the commits since the previous v* tag,
# minus the version bump commits, followed by the install note.
set -euo pipefail

tag="$1"
# A manual run can release a tag that doesn't exist yet; tauri-action creates it at HEAD.
ref="$tag"
git rev-parse -q --verify "refs/tags/$tag" >/dev/null || ref=HEAD
range="$ref"
if prev=$(git describe --tags --abbrev=0 --match 'v*' "$ref^" 2>/dev/null); then
  range="$prev..$ref"
fi

echo "## What's changed"
echo
git log --no-merges --reverse --format='- %s' "$range" | grep -v '^- Release v' || echo "- Maintenance release"
if [[ -n "${prev:-}" && -n "${GITHUB_REPOSITORY:-}" ]]; then
  echo
  echo "**Full changelog:** https://github.com/$GITHUB_REPOSITORY/compare/$prev...$tag"
fi

cat <<'EOF'

## Install

Apple Silicon only. The app is unsigned, so after copying it to Applications run:

```
xattr -dr com.apple.quarantine /Applications/ProxaLens.app
```
EOF
