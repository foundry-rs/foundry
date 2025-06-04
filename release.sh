#!/bin/bash

set -e  # exit on error.

### Check parameters for the new tag.

if [ -z "$1" ]; then
  echo "Error: No new TAG provided."
  echo "Example: $0 v1.1.0"
  exit 1
fi

TAG="$1"

### Verify and prepare.

git checkout master
git pull origin master

# git tag -d "$TAG" || true
# git push --delete origin "$TAG" 2>/dev/null || true

# cargo build --release

### Update CHANGELOG.md.

LATEST_TAG=$(git tag --list 'v*' | sort -V | tail -n 1) # get the latest tag.
git log --oneline --no-merges "$LATEST_TAG"..HEAD       # get the latest changes.
# TODO (@filip-parity): Update CHANGELOG.md with the latest changes.

### Update stable tag if needed.

# git push origin :refs/tags/stable           # delete the remote stable tag.
# git tag -fa stable -m "Update stable tag"   # create or move the local stable tag (force, annotated).
# git push origin --tags                      # push the new stable tag.

### Create and push version tag.

git tag -a "$TAG" -m "Created release tag $TAG" # create an annotated version tag.
git push origin "$TAG"                          # push the version tag.
