---
forge: patch
---

Redacted private key inputs in traces for the `signCompact`, `signWithNonceUnsafe`, `signKeychain`, `signKeychainAdmin`, and `signEd25519` cheatcodes. The `signWithNonceUnsafe` nonce is redacted as well, since a known nonce allows recovering the private key from the signature. Malformed calldata to a known cheatcode selector no longer falls back to raw calldata rendering.
