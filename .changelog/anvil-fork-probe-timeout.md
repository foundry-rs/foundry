---
anvil: patch
---

Bound fork identity probes to 500 ms, including retries, and treat timeouts as unavailable identity information so slow optional RPC methods do not hold up fork startup.
