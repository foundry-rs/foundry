---
anvil: patch
forge: patch
cast: patch
---

Bumped the Tempo dependencies past the T12 change that accepts trailing ABI bytes in precompile calls again. Anvil now validates the fee token balance instead of the native balance for every transaction on Tempo, so forked mainnet senders without native funds are no longer rejected, and new canary tests replay Tempo mainnet transactions from Relay, AA payout senders, an ERC-4337 bundler, an ERC-7821 relayer, and ERC-8021 attributed approvals under the newest hardfork.
