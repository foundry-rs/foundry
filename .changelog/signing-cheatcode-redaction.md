---
forge: patch
---

Redacted the `signWithNonceUnsafe` nonce in traces, since a known nonce allows recovering the private key from the signature, and stopped falling back to raw calldata rendering for malformed calldata with a known cheatcode selector.
