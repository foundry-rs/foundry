#!/bin/bash

# Wait for a live Anvil process to serve RPC, allowing time for upstream fork requests.
wait_for_anvil() {
  local pid="$1" port="$2" timeout="${3:-120}"
  local deadline=$((SECONDS + timeout))

  while (( SECONDS < deadline )); do
    if ! kill -0 "$pid" 2>/dev/null; then
      echo "ERROR: Anvil exited before serving RPC on port $port" >&2
      return 1
    fi
    if cast client --rpc-url "http://127.0.0.1:$port" --rpc-timeout 1 >/dev/null 2>&1; then
      echo "Anvil started successfully on port $port"
      return 0
    fi
    sleep 1
  done

  echo "ERROR: Anvil did not serve RPC on port $port within ${timeout}s" >&2
  return 1
}
