# Forge scripting internals

This guide describes the current contributor-facing `forge script` lifecycle, persisted resume
state, recovery contract, and known limitations. User-facing CLI instructions belong in the
[Foundry Book](https://getfoundry.sh).

## Ownership

| Stage                                                          | Owner                                                                                                                         |
| -------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| CLI options, network dispatch, and top-level transitions       | [`crates/script/src/lib.rs`](../../crates/script/src/lib.rs)                                                                  |
| Compilation, linking, resume loading, and signer reacquisition | [`crates/script/src/build.rs`](../../crates/script/src/build.rs)                                                              |
| Local execution and collection of broadcastable transactions   | [`execute.rs`](../../crates/script/src/execute.rs) and [`runner.rs`](../../crates/script/src/runner.rs)                       |
| On-chain simulation, metadata, and sequence construction       | [`simulate.rs`](../../crates/script/src/simulate.rs) and [`transaction.rs`](../../crates/script/src/transaction.rs)           |
| Preparation, signer selection, submission, and Tempo batching  | [`broadcast.rs`](../../crates/script/src/broadcast.rs)                                                                        |
| Pending-transaction reconciliation                             | [`progress.rs`](../../crates/script/src/progress.rs) and [`receipts.rs`](../../crates/script/src/receipts.rs)                 |
| Public artifacts and sensitive RPC data                        | [`crates/script-sequence`](../../crates/script-sequence) and [`multi_sequence.rs`](../../crates/script/src/multi_sequence.rs) |
| Post-deployment verification                                   | [`verify.rs`](../../crates/script/src/verify.rs)                                                                              |

Network selection follows the single-dispatch invariant in
[Custom EVM integrations](./networks.md). Recovery state contains network-specific transaction data,
but it must not become a second execution-family dispatcher.

## Lifecycle

`ScriptArgs::run_script` selects a concrete `FoundryEvmNetwork` and drives the typed state machine.
A new deployment follows this path:

```text
preprocess and compile
        |
        v
link -> prepare execution -> execute script
        |
        v
collect broadcastable transactions
        |
        v
simulate transactions against their RPCs
        |
        v
attach metadata and construct ScriptSequence values
        |
        v
reconcile saved pending hashes
        |
        v
prepare, sign or delegate, and submit remaining transactions
        |
        v
collect receipts -> save artifacts -> optionally verify
```

Local script execution and on-chain simulation are separate phases. `PreExecutionState::execute`
runs the Solidity script and collects transactions emitted by broadcast cheatcodes. Simulation
executes only that collected list against remote state. Publishing or resuming must preserve the
saved broadcast plan rather than rebuilding it from a new execution.

`--resume` still compiles so contract and verification metadata are available, but it loads the
saved sequence and skips simulation. It may execute the full script again to recover signers
introduced through script cheatcodes, so enabled FFI and other off-chain effects can run again.
Transactions from that execution must not replace or renumber the saved plan.

### Local execution safety and nonce assignment

`ScriptRunner` deploys required libraries and the script contract, calls `setUp()`, then calls the
selected script function. Broadcast cheatcodes collect externally publishable transactions during
those calls. Local execution adjusts the broadcast sender's nonce so CREATE addresses and later
transaction nonces match the intended on-chain sequence. An explicit `--sender-nonce` overrides the
initial provider-derived nonce.

Before deploying libraries, the runner sets the sender to that initial nonce. Regular CREATE
library deployments increment it through execution; CREATE2 library deployments use calls instead,
so the runner advances it explicitly by the number of generated library transactions. Deploying the
script contract with the default caller temporarily isolates and then restores that caller's nonce.
At each root `setUp()` or script-function call, the cheatcode inspector decrements the caller nonce
before execution so broadcasts observe the on-chain nonce. This correction is not triggered by the
first `vm.broadcast`, `vm.startBroadcast`, or `vm.getNonce` call.

Script execution protection is installed after deploying the script contract. Alongside the
`ADDRESS` check, the cheatcode inspector rejects `CALLER` in the main script's broadcasting frame
when its actual caller differs from the broadcast sender. Called contracts and deeper callbacks
retain their own caller semantics. Setting `script_execution_protection = false` disables both
checks. This is an opcode guard: it does not track sender values cached before broadcasting or
reads moved outside the broadcast by the compiler. Scripts should pass an explicit deployer
address when constructing transaction arguments.

The generated nonce is part of the operation plan. During sequential broadcasting,
`SendTransactionKind::prepare` waits for the provider nonce to reach the planned nonce and fails if
the provider has already advanced past it. Resume must reconcile the saved operation rather than
silently assigning the provider's next nonce, because changing a CREATE nonce changes its address.

## Compatibility sequence model

A `ScriptSequence` compatibility artifact stores:

- an ordered `transactions` queue containing requests or pre-signed envelopes plus display and
  verification metadata;
- separate `receipts`, `pending`, and `reverted` hash lists;
- chain ID, libraries, script returns, source commit, and timestamp;
- each transaction's RPC URL in a separate sensitive-cache file.

Each `TransactionWithMetadata` has an optional transaction hash. The authoritative recovery plan
wraps these transactions with stable operation IDs, immutable attempt records, and recovery state.
Ordinary recovery selects incomplete operations by their own hash associations. Batch recovery
advances only across a validated contiguous prefix whose transaction hashes have matching receipts,
so a receipt hole cannot skip an unfinished earlier batch operation.

During a new run, `FilledTransactionsState::bundle` creates one sequence for each consecutive run of
transactions using the same RPC. An A -> B -> A RPC order therefore produces three sequences, not
one sequence per endpoint or chain. A single sequence owns its public and sensitive paths. Multiple
sequences are wrapped in one `MultiChainSequence`; only the multichain container is written.

## Preparation and signer modes

Before `SendTransactionKind::prepare`, broadcasting selects the chain-specific transaction kind,
estimates fees, and applies general Tempo options. `prepare` may then synchronize the sender nonce,
re-estimate gas, resolve the Tempo fee token, convert Tempo account-abstraction creates, and attach
sponsorship. Those final values can differ from the request produced during simulation, so recovery
persists the fully prepared attempt before submission. A new operation may run preparation, but
resume does not prepare a replacement for an existing attempt.

| Mode                                | Submission                               | Durable checkpoint before submission   |
| ----------------------------------- | ---------------------------------------- | -------------------------------------- |
| Pre-signed envelope                 | `eth_sendRawTransaction`                 | Final encoded bytes and derived hash   |
| Local Ethereum wallet               | Sign, then `eth_sendRawTransaction`      | Final encoded bytes and derived hash   |
| Tempo account or session access key | Sign, then `eth_sendRawTransaction`      | Final encoded bytes and derived hash   |
| Browser wallet                      | Wallet-controlled signing and submission | Delegated request with no assumed hash |
| Unlocked RPC account                | `eth_sendTransaction`                    | Delegated request with no assumed hash |

The broadcaster persists each attempt before invoking its submission API. It records a definitive
hash in the transaction and `pending` afterward. For non-sequential chains it submits up to 100
transactions concurrently. Completion order is not necessarily plan order; stable operation IDs
associate attempts and responses with their planned transactions.

A signed send error leaves identical encoded bytes available for replay. An ambiguous browser or
unlocked error marks the delegated attempt `outcome unknown` and blocks automatic replacement;
only a definite local rejection clears that request. Resume never repeats preparation for an
existing attempt.

## Pending reconciliation and receipts

Before sending new work, `BundledState::wait_for_pending` checks each hash in `pending`. Sequences
from a multichain deployment are checked concurrently. Outside `--batch`, `--resume` first checks
the sequences in order in `CompiledState::resume`; a failing sequence does not stop the others,
each result is saved, and the errors are returned together, so this later check usually finds
nothing pending. In multichain runs, broadcast and reconcile errors name their chain, and the
sequence number when several sequences share it. A confirmed success removes the hash from
`pending` and appends its receipt. A revert removes the hash, records it in `reverted` rather than
`receipts`, which consumers treat as successful, and returns an error. A later `--resume` warns
about each recorded revert, never resubmits it, continues with the remaining operations, and
replaces the final success message with a warning listing the reverts. Receipt-watcher timeouts
keep retrying without consuming the retry budget while the selected RPC still returns the
transaction, so ordinary polling is unbounded; Tempo batches use `transaction_timeout`. If that
endpoint returns no transaction, the durable attempt remains the source of identity: signed bytes
may be replayed; delegated attempts with a known hash remain checkpointed for a later plain
`--resume`, while unknown outcomes remain blocked until explicitly resolved. Removing a hash from
`pending` this way prints an immediate `--resume` warning, so the guidance survives a later batch
failure. An ordinary broadcast that otherwise succeeds while a submitted operation still lacks a
receipt exits successfully, since a lagging or load-balanced endpoint can briefly return no
transaction, but it replaces the success message with a warning on stderr that lists those
operations.

An RPC receipt that repeatedly lacks block metadata follows a separate bounded retry path. If its
hash belongs to a durable attempt, reconciliation fails without removing it from `pending`;
otherwise the compatibility hash can be removed. Neither that incomplete receipt nor one endpoint
returning no transaction proves that a submission never reached the network, so neither condition
permits replacing an existing durable attempt. Ordinary legacy imports have no durable attempt and
can prepare a new submission.

Hashes are polled in operation order, because `pending` follows submission-response order and a
later nonce can remain visible indefinitely behind an earlier transaction that is gone. Polling
stops at the first transaction the endpoint no longer returns. A hash queued behind an earlier
operation on the same chain from the same sender, in any earlier sequence, that has neither a
receipt nor a pending hash, such as one removed by an earlier check, is not polled at all, so resume
reaches the replay of that operation. Each hash left unpolled is checked once: a receipt with the
requested confirmations is recorded, and a confirmed revert still fails the broadcast; a
transaction the endpoint no longer returns is handled like the first one; anything still visible
stays in `pending` for a later batch or `--resume`. The broadcaster skips an operation whose
receipt an earlier batch recorded while it was pending.

Receipts are sorted by block number and transaction index before persistence. Their order is not an
operation identity. Associate a receipt with a transaction by hash, as `BroadcastReader` does,
rather than by position.

## Persistence and resume

Single-chain runs use these latest-state files:

```text
broadcast/<script>/<chain>/<signature>-latest.json
cache/<script>/<chain>/<signature>-latest.json
```

Multichain runs use corresponding files under `broadcast/multi` and `cache/multi`. Timestamped
copies are created at selected saves. The cache contains RPC URLs and has owner-only permissions on
Unix; private keys are not persisted.

The authoritative state is a versioned owner-only `.recovery.json` snapshot beside the sensitive
cache file. Complete snapshots are atomically promoted, and a process-lifetime file lock excludes a
second writer. The public and sensitive JSON files are compatibility exports generated from that
snapshot; failure between their writes cannot roll authoritative recovery backward.

A new `--broadcast` refuses to replace a previous broadcast that submitted anything, through a
transaction hash or a saved attempt, and left an operation, on any chain, with neither a receipt nor
a recorded revert. Without a snapshot, progress written before snapshots existed is checked through
the broadcast export. A broadcast export written with a snapshot cannot show a submission whose
response was lost, so when its snapshot is missing, for example in a checkout without `cache/`, it
blocks while any operation lacks a receipt or recorded revert, even if nothing was sent. With
`--start-fresh`, it first moves the snapshot, including an interrupted pending commit, and both
exports to timestamped names. Dry runs never block, and neither do runs that never signed or sent
anything while their snapshot is present.

Resume obtains the current chain ID for a single-chain run and loads the authoritative snapshot,
restoring sensitive RPC URLs by stable operation position. Corrupt, inconsistent, or unsupported
generation-tagged state fails closed. A generationless legacy public/sensitive pair may be imported
after its pair-consistency checks pass. Batch import additionally validates transaction-hash,
pending, and receipt associations. Resume then:

1. stops, naming the attempt ID, while any delegated outcome remains unknown;
2. outside `--batch`, reconciles pending hashes, known delegated hashes, and saved signed attempts
   with mined receipts;
3. acquires signers, or re-executes the script, only for remaining unsigned requests without a
   saved signed payload or batch attempt;
4. derives remaining ordinary work by operation hash and batch work by a validated contiguous
   prefix, then prepares new attempts or replays saved signed bytes. `--batch` reconciles its saved
   attempt here, inside `broadcast_batch`.

A saved signed attempt that is visible but unmined and has no pending hash is not reconciled in
step 2. It needs no signer, but signers for other operations can be requested before it is
replayed.

The saved RPC is part of the sensitive sequence. A single-chain resume uses the `--rpc-url`
endpoint for Forge-managed reconciliation and submission, after checking that it reports the saved
chain, without rewriting the saved endpoint. Without `--rpc-url`, it uses the saved endpoint; a
configured `eth_rpc_url` or `FOUNDRY_ETH_RPC_URL` only selects the chain and sequence.
Browser-wallet submission stays wallet-controlled, and multichain resume keeps the saved endpoints
and warns when `--rpc-url` is passed.

## Tempo and multichain behavior

Regular Tempo transactions use the normal sequence path and populate network-specific fields across
the stages described in [Preparation and signer modes](#preparation-and-signer-modes). Their
recovery attempts preserve the final signed payload or delegated request before submission.

`--batch` is a separate, single-chain path. It converts all remaining operations into one Tempo type
`0x76` transaction, requires one sender, and preserves operation order as batch calls. The
authoritative recovery snapshot records one attempt shared by every operation in the batch before
submission. For locally signed and Tempo keychain submissions, it stores the final encoded payload
and derived hash; resume replays exactly those bytes without reacquiring the signer. For unlocked
submission, it stores the delegated request before calling the RPC. An ambiguous delegated outcome
is marked unknown and blocks automatic resubmission.

After a definitive hash is known, it is stamped onto every remaining transaction and saved once in
`pending`. One network receipt is copied per operation so existing artifact and verification
consumers retain their expected shape. A missing transaction or receipt timeout preserves the
durable attempt: signed attempts remain replayable, delegated attempts with a known hash are
reconciled by a later plain `--resume`, and unknown delegated outcomes require explicit operator
resolution. Delegated attempts are never replaced automatically.

Multichain mode stores per-chain sequences in one `MultiChainSequence`. Pending hashes are
reconciled per chain, while new sequences are broadcast in container order. There is no cross-chain
atomicity. Recovery must preserve independent per-chain progress and must not infer that one chain's
completion permits skipping an operation on another. With `--verify`, every sequence is verified
even after one fails, and the failures are returned together, each naming its chain.

## Known failure windows

| Window                                                                    | Recovery behavior or remaining limitation                                   |
| ------------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| A node accepts a transaction but its response is lost                     | A signed attempt is reconciled by hash; a delegated attempt fails closed    |
| The RPC returns a hash but the process exits before save                  | The pre-submission attempt remains durable and is reconciled conservatively |
| A receipt is observed but the process exits before save                   | Resume sees old pending state and rediscovers the receipt                   |
| A batch receipt hole precedes later confirmations                         | Batch resume rejects a receipt after an incomplete operation                |
| One RPC forgets a transaction or repeatedly returns an incomplete receipt | The attempt remains durable and replacement is not inferred                 |
| The process exits during a snapshot write                                 | Atomic replacement retains either the previous or new complete snapshot     |
| Two processes resume the same sequence                                    | The recovery lock excludes a competing writer                               |

## Recovery contract

The current broadcaster implements the following recovery contract for its authoritative snapshot.
Legacy broadcast and cache JSON remain compatibility exports for downstream consumers rather than
the source of recovery truth. The limitations and deferred work called out below remain outside the
implemented guarantee.

### Stable plan

- Persist a versioned, immutable operation plan before the first submission.
- Give every operation a stable `(sequence, operation)` ID within its recovery generation. IDs are
  independent of receipt order and process lifetime; resume preserves the saved sequence and
  operation order.
- Admit resume only when the snapshot's immutable plan matches the ordered operations saved in the
  same snapshot, including chain, batch membership, transaction fingerprints, RPC assignment, the
  requested `--batch` mode, and the snapshot's single- or multichain shape. Hashes, receipts, and
  pending progress are mutable snapshot state and do not define plan identity. The initial
  implementation does not persist build artifacts, signature arguments, or full execution
  configuration, and does not compare the plan with the current build: resume never rebuilds
  operations from those inputs, so changing them does not change the saved plan, but the change is
  not detected either.
- Never silently rebuild, renumber, omit, or insert operations during resume.
- Preserve batch membership and assign a stable batch ID when operations share one network
  transaction.

### Immutable attempts

- Model attempts separately from planned operations. Once prepared, an attempt's chain, sender,
  nonce, payload, fee fields, and signer mode are immutable.
- For locally signed submissions, persist final encoded bytes and the derived hash before sending.
  A retry sends exactly those bytes.
- For browser or unlocked signing, persist the delegation intent before invoking the external
  signer or RPC. If submission returns an ambiguous error, or the process exits before recording a
  definitive hash, preserve the intent and block automatic replacement; resume treats an
  interrupted prepared intent as `outcome unknown`. A recognized definite non-submission error
  instead clears the intent and fails the current run.
- Preserve pre-signed envelopes without converting them back into requests.

### Durable state

- Use one owner-only authoritative snapshot, or an equivalent transactionally published set.
- Publish complete snapshots with atomic replacement and fail closed on malformed, unsupported, or
  inconsistent state.
- The checkpoint guarantee covers process termination after successful publication. Host or power
  failure durability is outside this contract; guaranteeing it would additionally require syncing
  the snapshot and its parent directory before submission.
- Exclude competing writers for the same deployment. A reader must never observe a partial write.
- Generate public broadcast and sensitive compatibility artifacts from authoritative state;
  failure to update an export must not roll recovery state backward.
- Import generationless legacy artifacts only after validating the public/sensitive pair, then
  publish their recovery generation through the same recoverable replacement protocol. Missing,
  mixed, corrupt, or conflicting generation-tagged recovery state fails closed. Legacy batch
  hashes are imported only when every remaining operation and the pending set identify one hash;
  generation-tagged batch recovery never infers an attempt from incomplete mutable submission
  progress, so incomplete progress without a durable batch attempt is intentionally unsupported.

### Conservative reconciliation

- Reconcile known submissions before resolving signers, with the exceptions described in
  [Persistence and resume](#persistence-and-resume).
- Track completion per operation or batch by hash, not through receipt counts. A reverted outcome
  is recorded outside `receipts` and is never resubmitted.
- Absence from one RPC is not proof that a submission was never accepted. An invisible signed
  submission stays replayable with identical bytes and the run continues with a warning; batch
  disappearance and an invisible delegated submission with a known hash return an error.
- An unknown delegated outcome remains unresolved until an operator or supported chain query
  establishes its result. Resume names the attempt ID and refuses to reconcile or submit anything
  while it is unresolved. Once established, the operator targets that attempt with
  `--resume-attempt <ID>` and records a discovered hash with `--resume-tx-hash <HASH>`, or explicitly
  permits a retry with `--resume-retry` only after proving non-submission.
- Apply the configured confirmation count independently while reconciling each chain; this does not
  introduce separate per-chain policies. Revalidation of already persisted confirmations across a
  later reorg is outside this contract. Concurrent reconciliation must preserve each chain's state.

### Transaction semantics

For an existing attempt, recovery preserves every field that can change transaction identity or
behavior, including:

- chain ID, sender, nonce, destination or create form, value, input, and call order;
- gas limit and legacy or EIP-1559 fee fields;
- access lists, blobs, authorization lists, and pre-signed envelopes when present;
- Tempo fee token, nonce key, validity window, calls, access-key metadata, sponsor data, and batch
  membership.

An operation without an attempt is prepared on resume like a new one. Its saved request keeps the
sender, nonce, destination, value, input, and any recorded Tempo fee token. When `--batch` rebuilds
the remaining operations as one transaction, it uses the fee token recorded on the first remaining
operation. Fees, gas estimates, Tempo options such as the nonce key and validity window, and
sponsorship can come from the resume invocation, and `--tempo.fee-token` replaces a recorded fee
token. When the original run was sponsored, the recovery snapshot records the sponsor address, and
resume refuses to prepare remaining operations unless `--tempo.sponsor` names the same sponsor.

Reconciliation and rebroadcast of identical locally signed bytes do not request a signer. The
initial recovery implementation fails closed instead of replacing an existing attempt with changed
fees or validity fields. Replacement workflows are not supported.

### Handoff and compatibility

- Recovery state may contain public signed payloads and RPC information but never signer secrets.
- Pre-signed envelopes, saved signed payloads, and saved batch attempts do not require a signer for
  recovery. Other incomplete unsigned requests remain eligible for signer acquisition. Unknown
  delegated outcomes block resume before signer acquisition.
- A single-chain resume reaches the chain through an explicit `--rpc-url` instead of the saved
  endpoint, so a handoff does not need the original operator's RPC. Without the flag, it uses the
  saved endpoint. Multichain resume keeps the saved endpoints.
- Legacy batch-attempt import validates transaction, hash, pending, receipt, and sensitive-metadata
  associations. If it cannot prove a safe batch state, it fails closed rather than guessing.

## Further failure-injection coverage

Recovery tests belong around the real Forge executable, local nodes, and a controlled RPC proxy.
Additional coverage should inject process exit or transport failure:

- before forwarding a submission, after forwarding but before returning its response, after the
  response, and after each durable checkpoint;
- while several submission responses arrive out of order;
- with receipt holes and delayed confirmations;
- during temporary-file writes and publication, including a corrupt previous snapshot;
- during a Tempo sponsored transaction and a Tempo batch;
- independently on two chains in one multichain deployment;
- while a second process attempts to resume the same deployment.

Assertions cover final target state, nonce use, deployment addresses, operation-to-hash
associations, exact locally signed bytes, Tempo fields and batch membership, explicit unresolved
outcomes, and absence of duplicated or skipped operations.
