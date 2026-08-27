---
anvil: patch
forge: patch
foundry-common: patch
---

Accepted EIP-1474 `-32004` method-not-supported responses to the optional `anvil_nodeInfo` probe as method unavailability, so endpoints such as Hardhat can be forked, and made anvil's fork detection use the same classification as forge's.
