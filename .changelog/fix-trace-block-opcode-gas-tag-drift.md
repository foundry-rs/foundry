---
anvil: patch
---

Fixed `trace_blockOpcodeGas` on a forked node forwarding an unresolved block tag (`latest`/`pending`/`safe`/`finalized`) straight to the upstream RPC when the request falls at or before the fork point. The upstream node resolved the tag against its own current chain tip instead of the fork's snapshot, so a forked node whose local chain hadn't advanced past the fork point could silently return opcode-gas trace data for the wrong block. The resolved block number is now forwarded instead, matching `debug_accountInfoAt`'s existing behavior.
