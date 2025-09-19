alias forge-fmt="cargo r --quiet -p forge-fmt --"
forge-fmt-cmp() {
    cargo b --quiet -p forge-fmt || return 1
    forge_fmt_new="$(pwd)/target/debug/forge-fmt"

    tmp="$(mktemp -d)"
    in_f="$tmp/in.sol"
    cat < "/dev/stdin" > "$in_f"
    config=$1
    if [ -f "$config" ]; then
        cp "$config" "$tmp"
    else
        printf "[fmt]\n%s\n" "$config" > "$tmp/foundry.toml"
    fi

    pushd "$tmp" > /dev/null || return 1
    trap 'popd > /dev/null && rm -rf $tmp' EXIT

    forge fmt - --raw < "$in_f" > "$tmp/old.sol" || return 1
    "$forge_fmt_new" "$in_f" > "$tmp/new.sol" || return 1
    # echo -n "$(perl -pe 'chomp if eof' "$tmp/new.sol")" > "$tmp/new.sol" # chop last nl

    bat --paging=never "$tmp/old.sol" "$tmp/new.sol" || return 1

    difft --override='*:text' "$tmp/old.sol" "$tmp/new.sol" || return 1
}
