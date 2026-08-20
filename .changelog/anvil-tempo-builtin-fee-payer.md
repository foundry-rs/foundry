---
anvil: minor
---

Added a built-in Tempo fee payer to `anvil --network tempo`. `eth_signRawTransaction` sponsor-signs sender-signed Tempo AA transactions, and raw transactions carrying the sponsorship placeholder are sponsored automatically on submission, so `cast send --sponsor-url` and the Tempo SDK relay transport can point directly at anvil. The sponsor account defaults to the last dev account and is configurable via `--tempo.fee-payer`.
