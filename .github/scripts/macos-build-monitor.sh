#!/usr/bin/env bash
# Read-only samples: never include process arguments or environment variables.
set -u

sample() {
  printf '\n=== macOS build sample %s ===\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  uptime
  /usr/bin/vm_stat
  /usr/sbin/sysctl vm.swapusage
  /usr/bin/memory_pressure -Q
  df -k "$PWD"
  printf '\nTop resident-memory consumers (RSS/VSZ in KiB):\n'
  printf 'PID PPID CPU%% RSS VSZ STATE COMMAND\n'
  ps -axo pid=,ppid=,%cpu=,rss=,vsz=,state=,comm= | sort -k4,4nr | sed -n '1,20p'
  printf '\nCompiler, security agent, and runner processes:\n'
  ps -axo pid=,ppid=,%cpu=,rss=,vsz=,state=,comm= |
    awk '$0 ~ /rustc|cargo|\/ld$|HardenRunner|harden-runner|aegis|Runner\.Worker|Runner\.Listener/'
}

trap 'exit 0' TERM INT
while true; do
  # Preserve subsequent samples even if an individual diagnostic is unavailable.
  sample 2>&1
  if [[ "${1:-}" == "--once" ]]; then
    break
  fi
  sleep 10 &
  wait "$!" || break
done
