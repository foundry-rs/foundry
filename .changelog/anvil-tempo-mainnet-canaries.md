---
anvil: patch
forge: patch
cast: patch
---

Bumped the Tempo dependencies past the T12 change that accepts trailing ABI bytes in precompile calls again. Anvil's pool now runs Tempo's own transaction validation for every transaction on Tempo, resolving the fee token like execution does instead of requiring a native balance, so forked mainnet senders are no longer rejected, and new canary tests replay Tempo mainnet transactions from Relay, AA payout senders, an ERC-4337 bundler, an ERC-7821 relayer, and ERC-8021 attributed approvals under the newest hardfork.
