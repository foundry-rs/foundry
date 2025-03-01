# If you change anything in here, make sure to also adjust the lint CI job!
lint:
    just ensure-command cargo-nextest
    cargo fmt --all -- --check
    taplo format --check
    cargo clippy --tests --workspace -- -D warnings

format:
    just ensure-command taplo
    cargo fmt
    taplo format


# Ensures that one or more required commands are installed
ensure-command +command:
    #!/usr/bin/env bash
    set -euo pipefail

    read -r -a commands <<< "{{ command }}"

    for cmd in "${commands[@]}"; do
        if ! command -v "$cmd" > /dev/null 2>&1 ; then
            printf "Couldn't find required executable '%s'\n" "$cmd" >&2
            exit 1
        fi
    done
