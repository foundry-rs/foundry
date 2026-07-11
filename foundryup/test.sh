#!/usr/bin/env bash

# Tests for foundryup's `latest`/`stable` tag resolution: the `releases/latest`
# redirect resolver and its fallback to the GitHub API. `curl`/`fetch` are mocked
# so no network calls are made.

# Mocks/globals are used indirectly by the sourced foundryup functions.
# shellcheck disable=SC2329,SC2034,SC2317

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source foundryup for its function definitions without running an install.
export FOUNDRYUP_TEST=1
# shellcheck source=/dev/null
. "$SCRIPT_DIR/foundryup"

failures=0
api_marker=""
install_marker=""
sidecar_home=""
selector_tmp=""

check_eq() {
  local desc="$1" expected="$2" actual="$3"
  if [ "$expected" = "$actual" ]; then
    printf 'ok   - %s\n' "$desc"
  else
    printf 'FAIL - %s\n      expected: %q\n      actual:   %q\n' "$desc" "$expected" "$actual"
    failures=$((failures + 1))
  fi
}

# Silence progress/log output.
say() { :; }
warn() { :; }

check_eq "managed binaries include solar" \
  "forge cast anvil chisel solar" "${BINS[*]}"
check_eq "solar is optional for legacy releases" \
  "0" "$(is_optional_bin solar; echo $?)"

# --- resolve_latest_tag_via_redirect --------------------------------------

curl() { printf 'https://github.com/foundry-rs/foundry/releases/tag/v1.7.1'; }
check_eq "redirect success returns tag" \
  "v1.7.1" "$(resolve_latest_tag_via_redirect foundry-rs/foundry)"

curl() { printf 'https://github.com/foundry-rs/foundry/releases/latest'; }
check_eq "redirect without tag returns empty" \
  "" "$(resolve_latest_tag_via_redirect foundry-rs/foundry)"

curl() { return 1; }
check_eq "redirect curl failure returns empty" \
  "" "$(resolve_latest_tag_via_redirect foundry-rs/foundry)"

curl() { printf 'https://github.com/foundry-rs/foundry/releases/tag/not-a-version'; }
check_eq "redirect with non-version tag returns empty" \
  "" "$(resolve_latest_tag_via_redirect foundry-rs/foundry)"

# --- resolve_nightly_tag_via_feed ----------------------------------------

NIGHTLY_SHA="bc3b7f1e90bb7a30903d7d1ae2c532e4fba5679d"
OLDER_SHA="55210f8c412eac24e9318341efab1e27c5b03822"
# Minimal Atom feed: older entry first, then the newest nightly by updated
# timestamp and a stable tag.
FEED="<feed>
  <entry>
    <updated>2026-01-01T00:00:00Z</updated>
    <link href=\"https://github.com/foundry-rs/foundry/releases/tag/nightly-$OLDER_SHA\"/>
  </entry>
  <entry>
    <updated>2026-01-02T00:00:00Z</updated>
    <link href=\"https://github.com/foundry-rs/foundry/releases/tag/nightly-$NIGHTLY_SHA\"/>
  </entry>
  <entry>
    <updated>2026-01-03T00:00:00Z</updated>
    <link href=\"https://github.com/foundry-rs/foundry/releases/tag/v1.7.1\"/>
  </entry>
</feed>"

curl() { printf '%s' "$FEED"; }
check_eq "nightly feed returns newest nightly-<sha>" \
  "nightly-$NIGHTLY_SHA" "$(resolve_nightly_tag_via_feed foundry-rs/foundry)"
check_eq "nightly feed returns bounded unique candidates" \
  $'nightly-bc3b7f1e90bb7a30903d7d1ae2c532e4fba5679d\nnightly-55210f8c412eac24e9318341efab1e27c5b03822' \
  "$(FOUNDRYUP_NIGHTLY_CANDIDATE_LIMIT=5 resolve_nightly_tags_via_feed foundry-rs/foundry)"

FEED_WITH_COMPARE_LINKS="<feed>
  <entry>
    <updated>2026-01-01T00:00:00Z</updated>
    <link href=\"https://github.com/foundry-rs/foundry/releases/tag/nightly-$OLDER_SHA\"/>
    <content>compare nightly-$NIGHTLY_SHA...nightly-$OLDER_SHA</content>
  </entry>
  <entry>
    <updated>2026-01-02T00:00:00Z</updated>
    <link href=\"https://github.com/foundry-rs/foundry/releases/tag/nightly-$NIGHTLY_SHA\"/>
    <content>compare nightly-$OLDER_SHA...nightly-$NIGHTLY_SHA</content>
  </entry>
</feed>"
curl() { printf '%s' "$FEED_WITH_COMPARE_LINKS"; }
check_eq "nightly feed ignores compare links and sorts by updated" \
  $'nightly-bc3b7f1e90bb7a30903d7d1ae2c532e4fba5679d\nnightly-55210f8c412eac24e9318341efab1e27c5b03822' \
  "$(FOUNDRYUP_NIGHTLY_CANDIDATE_LIMIT=5 resolve_nightly_tags_via_feed foundry-rs/foundry)"

curl() { printf '%s' "$FEED"; }

curl() { printf '<feed><entry><link href="https://github.com/foundry-rs/foundry/releases/tag/v1.7.1"/></entry></feed>'; }
check_eq "nightly feed without nightly entry returns empty" \
  "" "$(resolve_nightly_tag_via_feed foundry-rs/foundry)"

curl() { return 1; }
check_eq "nightly feed fetch failure returns empty" \
  "" "$(resolve_nightly_tag_via_feed foundry-rs/foundry)"

selector_tmp="$(mktemp -d)"
trap 'rm -f "$api_marker" "$install_marker"; rm -rf "$sidecar_home" "$selector_tmp"' EXIT
mkdir -p "$selector_tmp/archive"
touch "$selector_tmp/archive/forge"
tar -czf "$selector_tmp/foundry.tar.gz" -C "$selector_tmp/archive" forge
download() {
  case "$1" in
    *"nightly-$OLDER_SHA"*) cp "$selector_tmp/foundry.tar.gz" "$2" ;;
    *) printf 'not a tar archive' > "$2"; return 0 ;;
  esac
}
PLATFORM=linux
ARCHITECTURE=amd64
EXT=tar.gz
FOUNDRYUP_REPO=foundry-rs/foundry
FOUNDRYUP_NIGHTLY_CANDIDATES=("nightly-$NIGHTLY_SHA" "nightly-$OLDER_SHA")
check_eq "nightly selector skips invalid archive candidate" \
  "nightly-$OLDER_SHA" "$(select_installable_nightly_tag)"
unset -f download

# --- resolve_version_and_tag (redirect + feed nightly) --------------------

API_JSON='{"tag_name": "v9.9.9", "name": "v9.9.9"}'
# Minimal `releases` listing used for the nightly API fallback path. The awk
# parser is line-oriented (it mirrors the real multi-line api.github.com JSON),
# so `tag_name` and `published_at` must be on separate lines.
NIGHTLY_API_JSON='[
  {
    "tag_name": "nightly-olderfeedfirst000000000000000000000000",
    "published_at": "2026-01-01T00:00:00Z"
  },
  {
    "tag_name": "nightly-deadbeef00000000000000000000000000000000",
    "published_at": "2026-01-02T00:00:00Z"
  }
]'

# `fetch` runs in a command-substitution subshell, so track calls via a file.
api_marker="$(mktemp)"
trap 'rm -f "$api_marker"' EXIT
fetch() { echo called >> "$api_marker"; printf '%s' "$API_JSON"; }

: > "$api_marker"
curl() { printf 'https://github.com/foundry-rs/foundry/releases/tag/v1.7.1'; }
FOUNDRYUP_VERSION="stable"
resolve_version_and_tag
check_eq "stable resolves via redirect" "v1.7.1" "$FOUNDRYUP_TAG"
check_eq "stable does not call API when redirect works" "0" "$(wc -l < "$api_marker" | tr -d ' ')"

: > "$api_marker"
curl() { return 1; }
FOUNDRYUP_VERSION="latest"
resolve_version_and_tag
check_eq "latest falls back to API tag" "v9.9.9" "$FOUNDRYUP_TAG"
check_eq "latest calls API when redirect fails" "1" "$(wc -l < "$api_marker" | tr -d ' ')"

: > "$api_marker"
curl() { printf '%s' "$FEED"; }
FOUNDRYUP_VERSION="nightly"
resolve_version_and_tag
check_eq "nightly resolves to newest feed tag" "nightly-$NIGHTLY_SHA" "$FOUNDRYUP_TAG"
check_eq "nightly does not call API" "0" "$(wc -l < "$api_marker" | tr -d ' ')"

: > "$api_marker"
curl() { return 1; }
fetch() { echo called >> "$api_marker"; printf '%s' "$NIGHTLY_API_JSON"; }
FOUNDRYUP_VERSION="nightly"
resolve_version_and_tag
check_eq "nightly falls back to API tag when feed fails" "nightly-deadbeef00000000000000000000000000000000" "$FOUNDRYUP_TAG"
check_eq "nightly calls API when feed fails" "1" "$(wc -l < "$api_marker" | tr -d ' ')"

# --- rust foundryup migration shim ----------------------------------------

# Sidecar binary path selection per platform.
uname() { echo "Linux"; }
check_eq "sidecar path on linux" \
  "$FOUNDRYUP_RUST_HOME/bin/foundryup" "$(rust_foundryup_bin)"
uname() { echo "MINGW64_NT-10.0"; }
check_eq "sidecar path on windows" \
  "$FOUNDRYUP_RUST_HOME/bin/foundryup.exe" "$(rust_foundryup_bin)"
unset -f uname

# Point the sidecar home at a temp dir and back ensure_rust_foundryup with a
# fake binary so no real install is attempted.
sidecar_home="$(mktemp -d)"
trap 'rm -f "$api_marker" "$install_marker"; rm -rf "$sidecar_home"' EXIT
FOUNDRYUP_RUST_HOME="$sidecar_home"
mkdir -p "$sidecar_home/bin"
fake_bin="$sidecar_home/bin/foundryup"
FOUNDRYUP_BOOTSTRAP_VERSION="0.0.5"

make_fake_bin() {
  printf '#!/usr/bin/env sh\necho "foundryup %s (abc 2020)"\n' "$1" > "$fake_bin"
  chmod +x "$fake_bin"
}

# `install_rust_foundryup` runs in a subshell-free context, so track calls via a file.
install_marker="$(mktemp)"
install_rust_foundryup() { echo called >> "$install_marker"; }

check_eq "sidecar_version reads installed version" \
  "0.0.5" "$(make_fake_bin 0.0.5; sidecar_version "$fake_bin")"

: > "$install_marker"; make_fake_bin "0.0.5"; ensure_rust_foundryup
check_eq "up-to-date sidecar skips reinstall" "0" "$(wc -l < "$install_marker" | tr -d ' ')"

: > "$install_marker"; make_fake_bin "0.1.0"; ensure_rust_foundryup
check_eq "newer sidecar is not downgraded" "0" "$(wc -l < "$install_marker" | tr -d ' ')"

: > "$install_marker"; make_fake_bin "0.0.4"; ensure_rust_foundryup
check_eq "older sidecar triggers reinstall" "1" "$(wc -l < "$install_marker" | tr -d ' ')"

: > "$install_marker"; rm -f "$fake_bin"; ensure_rust_foundryup
check_eq "missing sidecar triggers reinstall" "1" "$(wc -l < "$install_marker" | tr -d ' ')"

# A sidecar whose --version is unparsable or fails must reinstall, not abort.
: > "$install_marker"
printf '#!/usr/bin/env sh\necho "garbage output"\n' > "$fake_bin"; chmod +x "$fake_bin"
ensure_rust_foundryup
check_eq "unparsable sidecar version triggers reinstall" "1" "$(wc -l < "$install_marker" | tr -d ' ')"

: > "$install_marker"
printf '#!/usr/bin/env sh\nexit 3\n' > "$fake_bin"; chmod +x "$fake_bin"
ensure_rust_foundryup
check_eq "failing sidecar --version triggers reinstall" "1" "$(wc -l < "$install_marker" | tr -d ' ')"

# --- summary --------------------------------------------------------------

if [ "$failures" -ne 0 ]; then
  printf '\n%d test(s) failed\n' "$failures"
  exit 1
fi
printf '\nall tests passed\n'
