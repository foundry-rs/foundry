# Forge scripting internals

This guide describes the current contributor-facing `forge script` lifecycle, persisted resume
state and known limitations, followed by a proposed recovery contract. User-facing CLI instructions
belong in the [Foundry Book](https://getfoundry.sh).

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

## Current sequence model

A `ScriptSequence` currently stores:

- an ordered `transactions` queue containing requests or pre-signed envelopes plus display and
  verification metadata;
- separate `receipts` and `pending` hash lists;
- chain ID, libraries, script returns, source commit, and timestamp;
- each transaction's RPC URL in a separate sensitive-cache file.

Each `TransactionWithMetadata` has an optional transaction hash, but there is no stable operation
ID, attempt history, or per-operation outcome. Broadcasting uses `receipts.len()` as the start of
the remaining transaction suffix. That is correct only when successful receipts form a complete
prefix. A receipt hole can make resume skip an unfinished earlier operation and resend a later one.

During a new run, `FilledTransactionsState::bundle` creates one sequence for each consecutive run of
transactions using the same RPC. An A -> B -> A RPC order therefore produces three sequences, not
one sequence per endpoint or chain. A single sequence owns its public and sensitive paths. Multiple
sequences are wrapped in one `MultiChainSequence`; only the multichain container is written.

## Preparation and signer modes

Before `SendTransactionKind::prepare`, broadcasting selects the chain-specific transaction kind,
estimates fees, and applies general Tempo options. `prepare` may then synchronize the sender nonce,
re-estimate gas, resolve the Tempo fee token, convert Tempo account-abstraction creates, and attach
sponsorship. In-process retries reuse the request populated by broadcasting but run `prepare` again;
a fresh resume can repeat both stages. Final values can therefore differ from the request produced
during simulation.

| Mode                                | Submission                               | When the current implementation learns the hash   |
| ----------------------------------- | ---------------------------------------- | ------------------------------------------------- |
| Pre-signed envelope                 | `eth_sendRawTransaction`                 | From the RPC response, although locally derivable |
| Local Ethereum wallet               | Sign, then `eth_sendRawTransaction`      | From the RPC response, although locally derivable |
| Tempo account or session access key | Sign, then `eth_sendRawTransaction`      | From the RPC response, although locally derivable |
| Browser wallet                      | Wallet-controlled signing and submission | When the wallet returns a hash                    |
| Unlocked RPC account                | `eth_sendTransaction`                    | When the RPC returns a hash                       |

The broadcaster records the returned hash in the transaction and `pending`, then saves the
sequence. For non-sequential chains it submits up to 100 transactions concurrently. Completion
order is not necessarily plan order; the transaction index carried through each future associates a
successful response with its transaction.

On send errors, the current implementation can prepare and submit again. This is not a recovery
guarantee: a transport error can occur after acceptance, and repeated preparation can change gas,
sponsorship, a delegated signature, or other preparation-derived fields. A fresh resume can also
repeat earlier fee and network-specific filling.

## Pending reconciliation and receipts

Before sending new work, `BundledState::wait_for_pending` checks each hash in `pending`. Sequences
from a multichain deployment are checked concurrently. A confirmed success removes the hash from
`pending` and appends its receipt. A revert removes the hash and returns an error without appending
the receipt, which can leave a receipt hole. Receipt-watcher timeouts keep retrying without
consuming the retry budget while the selected RPC still returns the transaction. A hash becomes
eligible for another submission when that endpoint returns no transaction.

An RPC receipt that repeatedly lacks block metadata follows a separate bounded retry path and can
also remove the hash from `pending`. Neither that incomplete receipt nor one endpoint returning no
transaction proves that a submission never reached the network. Treat those states as known
limitations, not as general evidence that automatic replacement is safe.

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

These files are both compatibility artifacts and current resume state. They are written by
truncating the destination and serializing directly, public state before sensitive state. There is
no atomic publication of the pair, schema version, checksum, or process-level writer lock. Loading
rejects mismatched transaction and sensitive-metadata counts, but a crash or competing writer can
still leave malformed or semantically mixed state. `ScriptSequenceKind::drop` performs a final
best-effort save, which does not close those windows.

Resume obtains the current chain ID for a single-chain run and attempts to load the latest broadcast
and cache files, restoring sensitive RPC URLs by transaction position during loading. Any load
error, including missing or malformed JSON and mismatched sensitive metadata, falls back to the
dry-run sequence. The fallback retargets that sequence to the broadcast paths and immediately saves
it, potentially replacing the previous recovery files with an older simulation plan. It then:

1. reuses available signers or re-executes only to collect missing script-provided signers;
2. reconciles hashes currently listed in `pending`;
3. derives remaining work from the receipt-count suffix;
4. prepares and submits that remaining work.

The saved RPC is part of the sensitive sequence. Operator handoff therefore also hands off an
endpoint. The proposed recovery contract requires endpoint rebinding to verify the expected chain
identity before reconciliation or submission.

## Tempo and multichain behavior

Regular Tempo transactions use the normal sequence path and populate network-specific fields across
the stages described in [Preparation and signer modes](#preparation-and-signer-modes). The sequence
does not preserve an immutable copy of the final signed payload or every submission attempt.

`--batch` is a separate, single-chain path. It converts all remaining operations into one Tempo type
`0x76` transaction, requires one sender, and preserves operation order as batch calls. After the RPC
returns, the batch hash is stamped onto every remaining transaction and saved once in `pending`.
One network receipt is copied per operation so existing artifact and verification consumers retain
their expected shape.

The batch-specific recovery path checks an already stamped hash before resolving the batch signer
or sponsorship. End-to-end resume can nevertheless re-execute the script earlier to recover missing
script-provided signers. If recovery cannot find the transaction, it clears the checkpoint and
submits again in the same invocation. A recovery timeout clears the checkpoint and returns an error,
while a timeout immediately after a new submission retains the checkpoint for the next resume.
These ambiguous outcomes do not satisfy the proposed recovery contract below.

Multichain mode stores per-chain sequences in one `MultiChainSequence`. Pending hashes are
reconciled per chain, while new sequences are broadcast in container order. There is no cross-chain
atomicity. Recovery must preserve independent per-chain progress and must not infer that one chain's
completion permits skipping an operation on another.

## Known failure windows

| Window                                                                    | Current consequence                                                |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| A node accepts a transaction but its response is lost                     | No hash is checkpointed; generic retry can submit again            |
| The RPC returns a hash but the process exits before save                  | The network has an attempt absent from durable state               |
| A receipt is observed but the process exits before save                   | Resume sees old pending state and must rediscover it               |
| Concurrent operations confirm around an earlier receipt hole              | Receipt-count resume can select the wrong suffix                   |
| One RPC forgets a transaction or repeatedly returns an incomplete receipt | Current code can remove the hash and permit replacement            |
| The process exits during either JSON write                                | A file can be truncated or the two files can diverge               |
| Two processes resume the same sequence                                    | Both can reconcile and submit because there is no writer exclusion |

## Recovery contract

The following contract is a proposed, currently unimplemented architecture for durable recovery.
Stable operation and attempt IDs, authoritative recovery storage, signer-free reconciliation, and
fail-closed delegation are requirements for that implementation, not guarantees of the current
broadcaster. Legacy broadcast and cache JSON should remain useful to downstream consumers, but they
need not remain the authoritative recovery state.

### Stable plan

- Persist a versioned, immutable operation plan before the first submission.
- Give every operation a stable ID independent of receipt order, container position changes, and
  process lifetime. Scope identity by deployment and chain.
- Record as provenance the build and execution inputs that can change the ordered transaction
  requests, including build artifacts, selected signature and arguments, linked libraries, chain
  ID, sender, initial nonce, and execution-affecting configuration. A mismatch blocks rebuilding the
  saved plan or preparing new attempts, but not reconciliation or identical-byte rebroadcast of a
  persisted signed attempt. Verification-only settings, signer location, and validated endpoint
  handoff may change without invalidating the plan.
- Never silently rebuild, renumber, omit, or insert operations during resume.
- Preserve batch membership and assign a stable batch ID when operations share one network
  transaction.

### Immutable attempts

- Model attempts separately from planned operations. Once prepared, an attempt's chain, sender,
  nonce, payload, fee fields, and signer mode are immutable.
- For locally signed submissions, persist final encoded bytes and the derived hash before sending.
  A retry sends exactly those bytes.
- For browser or unlocked signing, persist the delegation intent before invoking the external
  signer or RPC. If control returns without a definitive hash, or the process exits before recording
  one, preserve the intent as `outcome unknown` and stop; do not request another attempt
  automatically.
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

### Conservative reconciliation

- Reconcile durable attempts before resolving signers or preparing new attempts.
- Track outcome per operation or batch, not through receipt counts.
- A known hash can become confirmed, reverted, still pending, explicitly replaced, or unresolved.
  Absence from one RPC is not proof that it was never accepted.
- An unknown outcome remains unresolved until an operator or supported chain query establishes its
  result. Resume reports affected operations and conservatively blocks later submissions in saved
  sequence and multichain-container order until the unresolved outcome is resolved. Operations in
  the same batch remain ordered together.
- Apply the configured confirmation count independently while reconciling each chain; this does not
  introduce separate per-chain policies. Revalidation of already persisted confirmations across a
  later reorg is outside this contract. Concurrent reconciliation must preserve each chain's state.

### Transaction semantics

Recovery preserves every field that can change transaction identity or behavior, including:

- chain ID, sender, nonce, destination or create form, value, input, and call order;
- gas limit and legacy or EIP-1559 fee fields;
- access lists, blobs, authorization lists, and pre-signed envelopes when present;
- Tempo fee token, nonce key, validity window, calls, access-key metadata, sponsor data, and batch
  membership.

Reconciliation and rebroadcast of identical locally signed bytes do not request a signer. Replacing
an attempt is a separate, explicitly authorized transition that may change fees or validity fields
and require a new signature. It must preserve the operation's intent and record both hashes.

### Handoff and compatibility

- Recovery state may contain public signed payloads and RPC information but never signer secrets.
- Another operator provides a signer only for operations with no signed or submitted attempt, or
  for an explicitly authorized replacement. Reconciliation and identical-byte rebroadcast do not
  need that signer, and an ambiguous delegated attempt remains blocked.
- Endpoint rebinding validates chain identity and retains original chain-scoped operation IDs.
- Legacy import validates transaction, hash, pending, receipt, and sensitive-metadata associations.
  If it cannot prove a safe state, it imports the operation as unresolved rather than guessing.

## Proposed failure-injection coverage

Tests for the proposed recovery contract belong around the real Forge executable, local nodes, and
a controlled RPC proxy. At minimum, inject process exit or transport failure:

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
