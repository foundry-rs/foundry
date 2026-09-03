---
cast: minor
forge: minor
---

Added optional Touch ID-assisted authentication for encrypted Cast keystores used by Cast and Forge on supported macOS builds. Packaged Apple Silicon releases enable Touch ID, while the packaged Intel macOS artifact retains its previous deployment target without the `touch-id` feature. Authentication may fall back to the macOS login password, and explicit keystore passwords remain supported. Cast can report, enroll, and remove Touch ID enrollment for existing keystores. Wallet listings hide recognized sidecars while preserving and deterministically ordering unknown files.
