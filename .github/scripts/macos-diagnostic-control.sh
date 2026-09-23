#!/usr/bin/env bash
# Control only this job's diagnostic processes; never print arguments or env.
set -euo pipefail
diagnostic_dir="${RUNNER_TEMP:?}/macos-diagnostic"
scripts_dir=$(cd "$(dirname "$0")" && pwd)

mark() {
  printf '%s %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$*" |
    tee -a "$diagnostic_dir/lifecycle.log"
}

stop_process() {
  local pid_file=$1 pid
  [[ -f "$pid_file" ]] || return 0
  pid=$(<"$pid_file")
  [[ "$pid" =~ ^[0-9]+$ && "$pid" -gt 1 ]] || return 1
  if kill -0 "$pid" 2>/dev/null; then
    # Children are the monitor's sleep or the workload's cargo/rustc processes.
    local child
    for child in $(pgrep -P "$pid" || true); do
      stop_tree "$child"
    done
    kill "$pid" 2>/dev/null || true
    for ((attempt=0; attempt<5; attempt++)); do
      kill -0 "$pid" 2>/dev/null || return 0
      sleep 1
    done
    kill -KILL "$pid" 2>/dev/null || true
  fi
}

stop_tree() {
  local pid=$1 child
  for child in $(pgrep -P "$pid" || true); do stop_tree "$child"; done
  kill -TERM "$pid" 2>/dev/null || true
}

case "${1:-}" in
  start)
    mark start-workload-step
    nohup bash "$scripts_dir/macos-diagnostic-worker.sh" \
      > "$diagnostic_dir/workload.log" 2>&1 < /dev/null &
    echo "$!" > "$diagnostic_dir/worker.pid"
    mark workload-detached
    ;;
  checkpoint)
    checkpoint=${2:?}
    [[ "$checkpoint" =~ ^[0-9]+$ ]] || exit 2
    mark "checkpoint-wait-start number=$checkpoint"
    # A finite polling step replaces tail -f and its EXIT-time wait.
    for ((poll=0; poll<60; poll++)); do
      [[ ! -f "$diagnostic_dir/build-exit.txt" ]] || break
      pid=$(<"$diagnostic_dir/worker.pid")
      if ! kill -0 "$pid" 2>/dev/null; then
        mark workload-disappeared-without-exit-marker
        printf '125\n' > "$diagnostic_dir/build-exit.txt"
        break
      fi
      sleep 5
    done
    mark "checkpoint-wait-end number=$checkpoint"
    if [[ -f "$diagnostic_dir/build-exit.txt" ]]; then
      echo 'DIAGNOSTIC_DONE=1' >> "${GITHUB_ENV:?}"
    fi
    snapshot="$diagnostic_dir/checkpoint-$checkpoint"
    mkdir -p "$snapshot"
    # Explicit allowlist: no environment, command arguments, raw runner logs,
    # source, build products, credentials, or tool caches in periodic artifacts.
    for file in resources.log network.log lifecycle.log machine.txt toolchain.txt build-exit.txt security-state.txt; do
      if [[ -f "$diagnostic_dir/$file" ]]; then
        cp "$diagnostic_dir/$file" "$snapshot/$file"
      fi
    done
    tail -60 "$diagnostic_dir/resources.log"
    if [[ -f "$diagnostic_dir/network.log" ]]; then tail -50 "$diagnostic_dir/network.log"; fi
    tail -30 "$diagnostic_dir/workload.log"
    mark "checkpoint-ready number=$checkpoint"
    ;;
  finish)
    mark cleanup-start
    stop_process "$diagnostic_dir/monitor.pid"
    stop_process "$diagnostic_dir/network-monitor.pid"
    if [[ ! -f "$diagnostic_dir/build-exit.txt" ]]; then
      mark workload-timeout-or-interruption
      stop_process "$diagnostic_dir/worker.pid"
      # Keep interruption distinct from a completed successful workload.
      printf '124\n' > "$diagnostic_dir/build-exit.txt"
    fi
    mark cleanup-end
    tail -60 "$diagnostic_dir/workload.log" || true
    status=$(<"$diagnostic_dir/build-exit.txt")
    [[ "$status" =~ ^[0-9]+$ && "$status" -eq 0 ]]
    ;;
  *) printf 'Usage: %s {start|checkpoint NUMBER|finish}\n' "$0" >&2; exit 2 ;;
esac
