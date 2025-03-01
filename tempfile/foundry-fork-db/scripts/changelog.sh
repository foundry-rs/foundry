#!/usr/bin/env bash
set -e -o pipefail

root=$(dirname "$(dirname "$0")")
cmd=(git cliff --workdir "$root" --output "$root/CHANGELOG.md" "$@")

if [ "$DRY_RUN" = "true" ]; then
    echo "skipping due to dry run: ${cmd[*]}" >&2
    exit 0
else
    "${cmd[@]}"
fi