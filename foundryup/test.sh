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
# Keep a copy of the real impl so it can be restored and tested later.
eval "real_install_rust_foundryup() $(declare -f install_rust_foundryup | sed '1d')"
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

# --- shim action classification -------------------------------------------

check_eq "shim_action update for -U" "update" "$(shim_action -U)"
check_eq "shim_action update among args" "update" "$(shim_action --install stable --update)"
check_eq "shim_action help for --help" "help" "$(shim_action --help)"
check_eq "shim_action version for -v" "version" "$(shim_action -v)"
check_eq "shim_action local for --list" "local" "$(shim_action --list)"
check_eq "shim_action local for --use" "local" "$(shim_action --use v1.0.0)"
check_eq "shim_action normal for install" "normal" "$(shim_action --install stable)"
check_eq "shim_action normal for bare" "normal" "$(shim_action)"
# Option values are skipped, not treated as commands.
check_eq "shim_action skips option values (update)" "normal" "$(shim_action --install --update)"
check_eq "shim_action skips option values (version)" "normal" "$(shim_action --branch --version)"
# The `--` sentinel ends scanning.
check_eq "shim_action stops at -- (update)" "normal" "$(shim_action -- --update)"
check_eq "shim_action stops at -- (list)" "normal" "$(shim_action -- --list)"
# Left-to-right: the first terminal flag wins (matches main's parser), so a
# leading terminal command is not overridden by a trailing --update and vice versa.
check_eq "shim_action version wins before update" "version" "$(shim_action --version --update)"
check_eq "shim_action update wins before version" "update" "$(shim_action --update --version)"
check_eq "shim_action list wins before update" "local" "$(shim_action --list --update)"
check_eq "shim_action use wins before update" "local" "$(shim_action --use v1.0.0 --update)"

# --- platform binary names ------------------------------------------------

uname() { echo "Linux"; }
check_eq "bin name on linux" "forge" "$(foundry_bin_name forge)"
uname() { echo "MINGW64_NT-10.0"; }
check_eq "bin name on windows" "forge.exe" "$(foundry_bin_name forge)"
unset -f uname

bin_dir="$(mktemp -d)"
: > "$bin_dir/forge"
uname() { echo "Linux"; }
check_eq "find_foundry_bin finds bare name" "$bin_dir/forge" "$(find_foundry_bin "$bin_dir" forge)"
: > "$bin_dir/cast.exe"
uname() { echo "MINGW64_NT-10.0"; }
check_eq "find_foundry_bin finds .exe on windows" "$bin_dir/cast.exe" "$(find_foundry_bin "$bin_dir" cast)"
unset -f uname
rm -rf "$bin_dir"

# --- nested versions layout (list / use) ----------------------------------

# Build a versions dir mixing the legacy flat layout and the nested layout a
# Rust foundryup migration produces.
FOUNDRY_VERSIONS_DIR="$(mktemp -d)"
mk_version() { # $1 = dir
  mkdir -p "$1"
  for bin in forge cast anvil chisel; do
    printf '#!/usr/bin/env sh\necho "%s 1.0.0"\n' "$bin" > "$1/$bin"
    chmod +x "$1/$bin"
  done
}
mk_version "$FOUNDRY_VERSIONS_DIR/v1.0.0"                       # flat
mk_version "$FOUNDRY_VERSIONS_DIR/foundry-rs/foundry/v2.0.0"    # nested

check_eq "is_version_dir true for version dir" \
  "yes" "$(is_version_dir "$FOUNDRY_VERSIONS_DIR/v1.0.0" && echo yes || echo no)"
check_eq "is_version_dir false for owner dir" \
  "no" "$(is_version_dir "$FOUNDRY_VERSIONS_DIR/foundry-rs" && echo yes || echo no)"

check_eq "resolve_version_dir finds flat version" \
  "$FOUNDRY_VERSIONS_DIR/v1.0.0" "$(resolve_version_dir v1.0.0)"
check_eq "resolve_version_dir finds nested version" \
  "$FOUNDRY_VERSIONS_DIR/foundry-rs/foundry/v2.0.0" "$(resolve_version_dir v2.0.0)"
check_eq "resolve_version_dir empty for missing version" \
  "" "$(resolve_version_dir v9.9.9)"

# list_versions_dir surfaces both flat and nested versions without erroring.
# Temporarily restore a printing `say` (the suite silences it) to capture output.
say() { printf "foundryup: %s\n" "$1"; }
list_out="$(list_versions_dir "$FOUNDRY_VERSIONS_DIR")"
say() { :; }
check_eq "list shows flat version" "yes" "$(printf '%s\n' "$list_out" | grep -qx 'foundryup: v1.0.0' && echo yes || echo no)"
check_eq "list shows nested version" "yes" "$(printf '%s\n' "$list_out" | grep -qx 'foundryup: v2.0.0' && echo yes || echo no)"
rm -rf "$FOUNDRY_VERSIONS_DIR"

# --- update_shim failure (best-effort) ------------------------------------

# A failed update-check must be non-fatal and must leave the on-PATH launcher
# untouched.
FOUNDRY_BIN_DIR="$(mktemp -d)"
FOUNDRY_BIN_PATH="$FOUNDRY_BIN_DIR/foundryup"
printf 'original-launcher\n' > "$FOUNDRY_BIN_PATH"

download() { return 1; }
rc=0; ( update_shim 2>/dev/null ) || rc=$?
check_eq "update_shim is non-fatal when download fails" "0" "$rc"

# A versionless download (e.g. a 404 body) is likewise non-fatal.
download() { printf '404: Not Found\n' > "$2"; return 0; }
rc=0; ( update_shim 2>/dev/null ) || rc=$?
check_eq "update_shim is non-fatal on versionless download" "0" "$rc"
check_eq "update_shim leaves launcher untouched on failure" \
  "original-launcher" "$(cat "$FOUNDRY_BIN_PATH")"
rm -rf "$FOUNDRY_BIN_DIR"

# --- install_rust_foundryup failure (best-effort) -------------------------

# Bootstrap failures must return nonzero (so migrate_and_exec can fall back),
# not abort the process.
install_rust_foundryup() { real_install_rust_foundryup "$@"; }  # restore real impl
FOUNDRYUP_RUST_HOME="$(mktemp -d)"

download() { return 1; }
rc=0; ( install_rust_foundryup "$FOUNDRYUP_RUST_HOME/bin/foundryup" 2>/dev/null ) || rc=$?
check_eq "install_rust_foundryup returns nonzero when download fails" "1" "$rc"

# A `404: Not Found` body downloads "successfully" but isn't a script; reject it.
download() { printf '404: Not Found\n' > "$2"; return 0; }
rc=0; ( install_rust_foundryup "$FOUNDRYUP_RUST_HOME/bin/foundryup" 2>/dev/null ) || rc=$?
check_eq "install_rust_foundryup rejects non-script installer body" "1" "$rc"
rm -rf "$FOUNDRYUP_RUST_HOME"

# --- migrate_and_exec serves local commands without bootstrapping ----------

# --help/--version are served by the shim; --list/--use reach the sidecar (if
# installed) or the legacy bash path. None of them may attempt a bootstrap, even
# when the network is unavailable.
mae_marker="$(mktemp)"
trap 'rm -f "$api_marker" "$install_marker" "$mae_marker"; rm -rf "$sidecar_home"' EXIT

FOUNDRYUP_RUST_HOME="$(mktemp -d)"
mkdir -p "$FOUNDRYUP_RUST_HOME/bin"
sidecar_bin="$FOUNDRYUP_RUST_HOME/bin/foundryup"
# A bootstrap or legacy-install attempt would call these; record so we can assert
# it never happens. The stubs that stand in for process-replacing/exiting calls
# exit the subshell so execution does not fall through to the bootstrap path.
install_rust_foundryup() { echo install >> "$mae_marker"; return 1; }
download() { echo download >> "$mae_marker"; return 1; }
main() { echo "main $*" >> "$mae_marker"; }
usage() { echo "usage" >> "$mae_marker"; exit 0; }
version() { echo "version" >> "$mae_marker"; exit 0; }
exec() { echo "exec $*" >> "$mae_marker"; exit 0; }

# --help/--version are handled in the shim: no sidecar, no main, no bootstrap.
: > "$mae_marker"
( migrate_and_exec --help )
check_eq "--help handled in shim, no bootstrap" "usage" "$(cat "$mae_marker")"

: > "$mae_marker"
( migrate_and_exec --version )
check_eq "--version handled in shim, no bootstrap" "version" "$(cat "$mae_marker")"

# No sidecar installed: --list/--use use the legacy bash path, no download.
: > "$mae_marker"
( migrate_and_exec --list )
check_eq "list without sidecar uses legacy main, no bootstrap" \
  "main --list" "$(cat "$mae_marker")"

# Sidecar installed: --list/--use exec it directly, no bootstrap.
: > "$mae_marker"
printf '#!/usr/bin/env sh\n:\n' > "$sidecar_bin"; chmod +x "$sidecar_bin"
( migrate_and_exec --list )
check_eq "list with sidecar execs it, no bootstrap" \
  "exec $sidecar_bin --list" "$(cat "$mae_marker")"

rm -rf "$FOUNDRYUP_RUST_HOME"

# --- summary --------------------------------------------------------------

if [ "$failures" -ne 0 ]; then
  printf '\n%d test(s) failed\n' "$failures"
  exit 1
fi
printf '\nall tests passed\n'
