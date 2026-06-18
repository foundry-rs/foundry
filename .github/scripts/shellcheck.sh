#!/usr/bin/env bash

# runs shellcheck and prints GitHub Actions annotations for each warning and error
# https://github.com/koalaman/shellcheck

IGNORE_DIRS=(
  "./.git/*"
  "./target/*"
)

ignore_args=()
for dir in "${IGNORE_DIRS[@]}"; do
  ignore_args+=(-not -path "$dir")
done

find . -name "*.sh" "${ignore_args[@]}" -exec shellcheck -f gcc {} + | \
  while IFS=: read -r file line col severity msg; do
    level="warning"
    [[ "$severity" == *error* ]] && level="error"
    file="${file#./}"
    echo "::${level} file=${file},line=${line},col=${col}::${file}:${line}:${col}:${msg}"
  done

exit "${PIPESTATUS[0]}"
