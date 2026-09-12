---
anvil: patch
anvil-core: patch
cast: patch
chisel: patch
forge: patch
forge-doc: patch
forge-fmt: patch
forge-lint: patch
forge-script: patch
forge-script-sequence: patch
forge-verify: patch
foundry-bench: patch
foundry-cheatcodes: patch
foundry-cli: patch
foundry-common: patch
foundry-common-fmt: patch
foundry-debugger: patch
foundry-evm: patch
foundry-evm-abi: patch
foundry-evm-core: patch
foundry-evm-coverage: patch
foundry-evm-fuzz: patch
foundry-evm-symbolic: patch
foundry-evm-traces: patch
foundry-primitives: patch
---

Fixed Windows builds by removing Unix-only consensus runtime dependencies from the Tempo EVM dependency graph.
