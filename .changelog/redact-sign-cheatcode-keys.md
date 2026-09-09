---
forge: patch
---

Fixed private keys being shown in verbose test traces for `signCompact`, `signWithNonceUnsafe`, `signKeychain`, `signKeychainAdmin`, and `signEd25519`; their key arguments are now redacted like `sign` and `signP256`.
