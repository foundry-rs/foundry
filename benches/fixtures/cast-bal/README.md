# Frozen Cast BAL panel

This directory holds public Ethereum history recorded from `ethereum.reth.rs`
for a reproducible local benchmark. `panel.json` identifies six transactions:
first, middle, and last positions in blocks 26011850 (417 transactions) and
26011861 (264 transactions). The panel was captured on 2026-09-19, from a frozen
20-block candidate interval ending at finalized block 26011867. It represents
the Ethereum Bpo2 period only.

`manifest.json` records SHA-256 hashes for the panel and the compressed RPC
recordings. Each `block-*.json.gz` contains JSON with `schema_version`,
`block_number`, `parent_hash`, and a list of `{method, params, result}` records.
The source journal hashes identify the preparation recordings from which these
fixtures were extracted. Gzip timestamps are zero for deterministic storage.

The captures contain block/transaction/receipt metadata, the authentic BAL,
and independently recorded account/storage values at the real parent block.
The BAL supplies prefetch addresses and keys; its changed values never supply
the parent state. The transport maps equivalent parent block number/hash
selectors, normalizes hex casing and storage-key padding, and projects captured
account-info fields into balance/code/nonce calls. It can also reconstruct
account-info only when all three fields were recorded at that same parent.
Conflicting records, missing responses, or requests for other state blocks fail.

The benchmark starts a fresh Anvil instance per block and disables disk caches.
Both Cast revisions prepare all cases before the fixture server closes its
barrier. All subsequent validation, warmup, and measured attempts must run
without a single fixture request. There is no external endpoint or credential
in the runtime path. Optional `eth_getAccountInfo` is disabled at the Cast
gateway equally for both revisions and both modes.

To replace this panel, capture a new set of finalized block ancestry, receipts,
BALs, and authentic parent-state RPC responses; retain their provenance hashes,
update the panel and manifest together, and run the complete campaign with real
binaries. Do not fill missing records with guessed or post-transaction values.

Verification:

```bash
python3 benches/scripts/test_cast_bal_campaign.py
```
