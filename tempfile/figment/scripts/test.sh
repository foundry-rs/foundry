#!/usr/bin/env bash
set -e

SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

function relative() {
  local full_path="${SCRIPT_DIR}/../${1}"

  if [ -d "${full_path}" ]; then
    # Try to use readlink as a fallback to readpath for cross-platform compat.
    if command -v realpath >/dev/null 2>&1; then
      realpath "${full_path}"
    elif ! (readlink -f 2>&1 | grep illegal > /dev/null); then
      readlink -f "${full_path}"
    else
      echo "Figment's scripts require 'realpath' or 'readlink -f' support." >&2
      echo "Install realpath or GNU readlink via your package manager." >&2
      echo "Aborting." >&2
      exit 1
    fi
  else
    # when the directory doesn't exist, fallback to this.
    echo "${full_path}"
  fi
}

# Root of workspace-like directories.
PROJECT_ROOT=$(relative "") || exit $?

# Add Cargo to PATH.
export PATH=${HOME}/.cargo/bin:${PATH}
CARGO="cargo"

# Ensures there are no tabs in any file.
function ensure_tab_free() {
  local tab=$(printf '\t')
  local matches=$(git grep -PIn "${tab}" "${PROJECT_ROOT}" | grep -v 'LICENSE')
  if ! [ -z "${matches}" ]; then
    echo "Tab characters were found in the following:"
    echo "${matches}"
    exit 1
  fi
}

# Ensures there are no files with trailing whitespace.
function ensure_trailing_whitespace_free() {
  local matches=$(git grep -PIn "\s+$" "${PROJECT_ROOT}" | grep -v -F '.stderr:')
  if ! [ -z "${matches}" ]; then
    echo "Trailing whitespace was found in the following:"
    echo "${matches}"
    exit 1
  fi
}

if [[ $1 == +* ]]; then
    CARGO="$CARGO $1"
    shift
fi

echo ":: Checking for tabs..."
ensure_tab_free

echo ":: Checking for trailing whitespace..."
ensure_trailing_whitespace_free

echo ":: Updating dependencies..."
if ! $CARGO update ; then
  echo "   WARNING: Update failed! Proceeding with possibly outdated deps..."
fi

if [ "$1" = "--core" ]; then
  FEATURES=(
    env
    json
    yaml
    test
  )

  echo ":: Building and testing core [no features]..."
  $CARGO check --no-default-features

  for feature in "${FEATURES[@]}"; do
    echo ":: Building and testing core [${feature}]..."
    $CARGO check --no-default-features --features "${feature}"
  done
else
  echo ":: Building and testing libraries..."
  $CARGO test --all-features --all $@
fi
