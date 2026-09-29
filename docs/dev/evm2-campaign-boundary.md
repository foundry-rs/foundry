# Proposed execution boundary for Forge campaigns

Proposal for [OSS-1030](https://linear.app/tempoxyz/issue/OSS-1030), informed by
[OSS-833](https://linear.app/tempoxyz/issue/OSS-833). This proposes part of the shared execution
design; it is not an API already agreed with the foundation owner. Names below are illustrative.
The campaign-facing contract does not settle every hook or active-frame operation needed by
Forge, Script, Cast and Chisel.

## Recommendation

Use a Foundry-owned session with **deferred acceptance of each call**. Execution produces a
pending call that exclusively borrows that session. The caller can inspect observations and
test-failure facts, then consume the pending call by accepting or discarding it. Engine state,
environment and retained cheatcode state stay inside the session. A returned report cannot be
committed later or applied to another session.

Keep generators, fixture decoding, target selection, run scheduling, shrinking, corpus storage
and failure policy in their existing Foundry owners. Adapt those consumers to the session;
do not grow the temporary native proptest loop into a second campaign implementation.

## What the existing campaigns require

Source baseline: master `7f3183c338a20a992f438d4a228d2c422ee365f0`. Foundation comparison:
`22faeddbf110284192635848d3d0cc5453d3c573`. Relevant paths below are shared between them except
where noted. The [initial parity results](./evm2-campaigns.md#initial-differential-results)
are runtime evidence for those specific tests, not proof of the proposed interface.

| Consumer | Existing behavior | Required boundary capability |
| --- | --- | --- |
| Stateless fuzz | `FuzzedExecutor::single_fuzz` makes a speculative call, classifies assumptions/skips and failures, collects feedback, and discards state. | Observe discarded execution; reset case state without resetting the campaign dictionary or generator. |
| Invariant handler | `FuzzCampaign::run_sequence` collects feedback before accepting non-assumption calls, including ordinary reverts. Failure policy is evaluated separately. | Inspect before acceptance; acceptance is not synonymous with EVM success or test success. |
| Delayed handler | `execute_invariant_tx` applies warp/roll and clamps value to balance; rejection/cancellation restores the delayed environment. | Stage environment with the call; return the effective input that actually ran. |
| Invariant predicates and `afterInvariant` | `call_invariant_function` and `call_after_invariant_function` use speculative calls on current sequence state. | Read accepted handler state, discard predicate writes, retain diagnostics. |
| Fixture and target getters | Forge reads these with speculative ABI calls. Filtering determines which fixtures are evaluated. | Ordinary speculative calls; no new engine-level fixture or target-selection API. |
| Shrinking and counterexamples | Candidates start from the baseline and execute ordered sequences; failure identity and replay metadata constrain acceptance. | Repeatable session reset and the same call primitives, with replay policy retained by the runner. |
| Corpus/showmap | Each entry starts from a fresh executor; stateful entries retain steps, stateless entries do not. | Entry-local session state and independently owned per-call coverage. |

Source owners: [`campaign.rs`](../../crates/evm/evm/src/executors/campaign.rs),
[`fuzz/mod.rs`](../../crates/evm/evm/src/executors/fuzz/mod.rs),
[`invariant/mod.rs`](../../crates/evm/evm/src/executors/invariant/mod.rs),
[`invariant/shrink.rs`](../../crates/evm/evm/src/executors/invariant/shrink.rs),
[`showmap.rs`](../../crates/evm/evm/src/executors/showmap.rs), and
[`ContractRunner::fuzz_fixtures`](../../crates/forge/src/runner.rs).

## Proposed surface and borrowing rules

Conceptual Rust signatures, not a compiling patch or a new trait hierarchy:

```rust,ignore
impl ExecutionSession {
    fn checkpoint(&self) -> SessionCheckpoint;
    fn restore(&mut self, checkpoint: &SessionCheckpoint) -> Result<()>;
    fn execute(&mut self, call: CallRequest) -> Result<PendingCall<'_>>;
}

impl SessionCheckpoint {
    fn spawn_session(&self) -> Result<ExecutionSession>;
}

impl PendingCall<'_> {
    fn report(&self) -> &CallReport;
    fn test_facts(&mut self, target: Address) -> Result<TestFacts>;
    fn feedback(&self) -> ExecutionFeedback<'_>;
    fn accept(self) -> Result<CallReport>;
    fn discard(self) -> CallReport;
}
```

### What the call sites would look like

These are design snippets using the proposed types, not compiling production code. `campaign`
below is the existing runner's generation, feedback and classification logic. It is separate
from `session`, so inspecting feedback does not require a second borrow of execution state.

A stateless fuzz case always discards execution state, even when the test passes:

```rust,ignore
let mut pending = session.execute(call)?;
if pending.report().is_cancelled() {
    let report = pending.discard();
    return campaign.cancel(report);
}
if pending.report().is_assumption_rejected() || pending.report().skip_reason().is_some() {
    let report = pending.discard();
    return campaign.reject_or_skip(report);
}

let facts = pending.test_facts(test_address)?;
campaign.observe(pending.feedback());
let report = pending.discard();
campaign.classify_fuzz_case(report, facts)
```

An invariant handler's acceptance decision is separate from its failure classification. Here,
`observe_handler` preserves the existing feedback-before-assumption-classification ordering:

```rust,ignore
let mut pending = session.execute(handler_call)?;
if pending.report().is_cancelled() {
    let report = pending.discard();
    return campaign.cancel(report);
}
campaign.observe_handler(pending.feedback());
if pending.report().is_assumption_rejected() {
    let report = pending.discard();
    return campaign.reject_handler(report);
}

let facts = pending.test_facts(handler_address)?;
let report = pending.accept()?; // Ordinary EVM reverts also reach this line.
campaign.classify_handler(report, facts); // Applies fail_on_revert and assertion policy.
// The existing scheduler decides whether to check predicates, continue, or stop.
```

Predicates see accepted handler effects, but their own writes are discarded. Every shrink
candidate starts from the same checkpoint; candidate generation remains outside execution:

```rust,ignore
let baseline = prepared_session.checkpoint();
let mut session = baseline.spawn_session()?;

for candidate in shrink_candidates {
    session.restore(&baseline)?;
    // Reuses the existing sequence policy and the handler operation above.
    campaign.replay_candidate(&mut session, &candidate)?;

    let mut pending = session.execute(invariant_call.clone())?;
    let facts = pending.test_facts(invariant_address)?;
    let report = pending.discard();
    campaign.consider_candidate(candidate, report, facts);
}
```

The lifetime is the ownership constraint, not just documentation:

```rust,ignore
struct PendingCall<'session> {
    session: &'session mut ExecutionSession,
    // Private speculative state/environment/cheatcodes and observations.
}

let pending = session.execute(call)?;
session.restore(&baseline)?; // Must not compile: pending still borrows session.
let report = pending.accept()?;
// The borrow ends here. The owned report has no accept/commit operation.
```

No engine-backed storage layout is proposed by that last struct. Acceptance atomically installs
the speculative successor; explicit discard and drop leave accepted session state intact.
Each feedback consumer must finish borrowing, or copy only what it retains, before resolving
the pending call. The existing replay consumers still keep their distinct policies described
below; `replay_candidate` does not imply a new universal replay algorithm.

Construct the session through the shared tool builder from its resolved execution family,
backing database/fork, environment and enabled inspectors. Campaign callers do not select a
second engine or reconstruct network context. Setup/deployment accepts calls on this session;
the runner captures a checkpoint at the appropriate preparation boundary.

`CallRequest` describes a Foundry synthetic call: caller, target, calldata and value, with
optional block/time deltas. Canonical historical transaction execution remains a distinct
entry point with its own validation and fee policy. Do not turn campaign artifacts into signed
network transactions or silently apply synthetic relaxations to canonical replay.

Resolve campaign-specific value clamping through session-owned balance reads, before execution;
do not clamp every shared executor call. Return the effective caller/target/calldata/value and
applied deltas in the report so persistence never records an unexecuted requested value.

There is at most one pending call per worker session. While it exists, the caller cannot execute
another top-level call, restore a checkpoint, or mutate the session directly. `accept`/`discard`
consume it; dropping it also discards session-local speculative effects. Backend errors and
cancellation leave the prior accepted session intact. A cancelled execution cannot be accepted.
Read caches may remain shared only where doing so cannot change execution semantics.

`test_facts` performs any required legacy `failed()` query against the prospective post-call
state without accepting it. Its internal probe must not change that state, append probe traces
to the original call, consume campaign RNG, or reborrow the outer session unsafely. This is a
Foundry adapter operation, not a field on evm2's `TxResult`; its safe implementation needs a probe.

For callbacks, the session lends a short-lived operation handle to the active hook. It exposes
only the state/environment and child-execution operations needed by that hook, not a second
mutable session or general mutable journal. A nested call must release/split the active
inspector borrow before reentering the dispatcher. Exact capabilities and callback order remain
joint design work with OSS-1033/1042; the pending top-level handle is not a solution to reentrancy
by itself. Keep campaign callback state separate from the session so feedback can be consumed
without borrowing both through one mutable runner object.

## State ownership and checkpoints

The session owns accepted state, fork identity/backing databases, environment/network context,
cheatcode state and snapshot bookkeeping. The pending operation owns only a speculative successor;
it is never a second accepted state owner. Acceptance installs all retained components together.
Discard restores them together, including pre-call warp/roll deltas and host-side overrides.
This does not promise rollback of filesystem/FFI side effects already permitted by cheatcodes.

Recommend an internal infallible in-memory acceptance path, with fallible reads and validation
completed before installation. If backend export can fail, stage it before publishing either
accepted layer; never mutate a fallible sink incrementally and then report a clean rollback.
OSS-1040 must select the concrete storage strategy. Whether evm2 retains the accepted overlay or
exports into a Foundry backend is private to the session; campaign code should not depend on it.

Campaign checkpoints are taken **between executions**, after setup and applicable per-test
preparation. They include the matching backing fork/block, accepted writes, environment,
retained cheatcode state and session snapshot registry. Restoring them must not pair cached
accounts from one fork/block with another database. Spawned workers may share immutable/COW
data and RPC caches, but not mutable logical case state. Require a detached worker-transferable
checkpoint; do not require a live EVM or inspector borrow to be `Send + Sync`.

These checkpoints are separate from the in-frame `vm.snapshotState` contract in OSS-1041.
An idle checkpoint does not establish active-frame journal, warmth, log-retention or sticky
assertion semantics. Campaign dictionaries, dynamic target indexes, coverage history, run
counters and generator RNG stay outside the checkpoint and follow their existing reset rules.
Cheatcode RNG remains session-owned and needs a narrow seed/reset operation; retain existing
seed/worker/run derivation and persisted replay metadata in the campaign layer.

## What a report must contain

Do not export `RawCallResult` with its mutable REVM state, entire cheatcode object and environment
as the lasting contract. Split observations from state acceptance:

| Data | Required content and purpose |
| --- | --- |
| Execution outcome | Success, revert, exceptional halt or cancellation; raw output and engine-independent halt classification. Host/database errors remain errors, not Solidity counterexamples. |
| Test facts | Snapshot assertion failure, current-call versus pre-existing global failure, legacy assertion result, assertion diagnostics, and authenticated skip provenance. Do not reduce these to one universal `success` boolean. |
| Failure location | Top-level handler target/selector and innermost reverter separately; preserve the data used for failure fingerprints and decoding. |
| Gas and environment observations | Used/refunded gas and stipend with explicit meanings; effective input and fork/block identity. Full mutable environment stays private. |
| Diagnostics | Logs, labels, optional traces/debug bytecodes, deprecations and breakpoints. Observation ownership must survive either accept or discard. |
| Campaign feedback | Coverage, comparison operands, observed subcalls, dictionary values/mapping observations and candidate contract bytecode. Collect only enabled channels. |

Test classification stays in Foundry. Normal tests inspect global failure; invariant handler
gates and predicates can ignore a stale committed failure from an already recorded bug.
`is_success_handler_gate` and `did_fail_on_assert` encode different checks today. Preserve both,
including logged/decoded assertion evidence, rather than treating a non-reverting call as passing.

Expose dictionary/target feedback as borrowed views or visitors while pending; detach only the
data a consumer needs to retain. Account code and storage observations are **not just writes**:
`insert_new_state_values` currently sees account information and observed storage in the REVM
changeset. `collect_created_contracts` uses touched accounts with nonempty code, not exclusively
CREATE events. Exporting only evm2 `PendingState` writes or newly-created addresses would narrow
those inputs. Specify/test equivalent observation selection and ordering before changing them;
the generator owns ABI decoding, dictionary insertion and target filtering.

Optional inspection must also support the existing internal call generator and its recorded
inner-sequence replay. This is a bounded inspector integration requirement, not permission for
campaign code to receive a mutable EVM context. Keep replay sequences and generator ownership in
the campaign layer, and settle the reentrant hook borrowing with OSS-1033.

## Acceptance policy remains with each workflow

| Workflow | Resolve pending call | Policy after resolution |
| --- | --- | --- |
| Stateless case | Discard, including passing cases | Existing assumption/skip rules, `fail_on_revert`, counts, feedback and persistence |
| Invariant handler | Discard assumption rejection/cancellation; accept other completed calls, including reverts | Existing handler failure classification, predicate cadence and sequence retention |
| Predicate / `afterInvariant` | Discard | Evaluate facts against current sequence; do not contaminate later handlers |
| Shrink candidate / persisted replay | Start from baseline and apply that workflow's existing step/check rules | Preserve failure identity, full sequence, delays and metadata; never silently start fresh fuzzing |
| Corpus/showmap entry | Fresh baseline per entry; stateful versus stateless disposition follows replay policy | Preserve filtering, replay counts, coverage format and artifact paths |

Do not claim every replay loop currently has identical rejection semantics. For example,
`showmap.rs` explicitly filters assumptions/skips, while corpus synchronization and diagnostic
`replay_run` use other paths. The session supplies mechanisms; unifying those policies would be
a separate behavior change requiring master fixtures, not incidental cleanup during the port.

## Mapping to evm2 and implementation order

At foundation's pinned evm2 revision `6eb1d262cd6049a9e709e517e97a0bea8877b9f9`,
`ExecutedTx::{result,commit,discard,commit_with,discard_with,detach}` supplies deferred transaction
resolution. `commit` accepts into evm2's internal overlay; `detach` does not. These are useful
internal primitives, not complete Foundry session operations: they do not own all fork,
environment, inspector and diagnostic state. `State::snapshot`/`StateSnapshot::into_state`
provides detached engine state, not the full Foundry checkpoint. This is source evidence for
that pin, not a claim about latest upstream or a validated Foundry implementation.

1. Agree the pending-call ownership model and test-fact/feedback split in OSS-1030. Avoid a
   generic replacement for `FoundryContextExt`/`ContextTr`; start with concrete Foundry types.
2. In OSS-823/1040, prove accept/discard and error atomicity with two sequential calls. In
   OSS-1041, prove checkpoint reset/spawn with local and fork reads plus environment/cheatcodes.
3. Prove pending-state assertion inspection, cancellation and rejected delayed calls. Port the
   observation adapters with OSS-834, including dictionary inputs and dynamic target discovery.
4. Adapt existing stateless campaigns, then invariant sequences and their shrink/replay consumers.
   Keep serialization outside the session and retire the temporary native loop as each path lands.

Required proofs include `assumption_rejection_restores_delayed_block_environment`,
`handler_vm_assert_global_flag_does_not_poison_invariant_checks`, dynamic target replay, and the
existing [parity fixture map](./evm2-campaigns.md#parity-fixtures). A failed-export probe is still
required; it is not established by the initial fuzz tests.

The standalone [`InvariantSessionBoundary.t.sol`](../../testdata/fixtures/InvariantSessionBoundary.t.sol)
probe passed on the pinned master binary with Solc 0.8.30, seed 1, one worker, three runs and
depth four (12 handler calls, zero reverts). Its handler checks that predicate writes never leak
and that a previous run's count was reset; `afterInvariant` checks all four handler writes were
retained. Copy it into an otherwise empty Forge project's `test/` directory and run:

```sh
FOUNDRY_INVARIANT_RUNS=3 FOUNDRY_INVARIANT_DEPTH=4 FOUNDRY_INVARIANT_FAIL_ON_REVERT=true \
  forge test --use 0.8.30 --fuzz-seed 1 -j 1
```

This establishes a master requirement, not evm2 support. The fixture intentionally uses only
the required targeting getters; warnings about absent optional getters are expected.

Two decisions to take to mablr: **agree on deferred accept/discard as the session primitive**, and
**agree that reports expose Foundry observations/test facts rather than mutable engine state**.
The remaining work is to prove the internal ownership and hook details, not pick another existing
executor type to copy. An immediate `transact` convenience can resolve this primitive internally;
an up-front commit flag alone cannot support the campaign's post-execution decision.

This proposal changes no public API or artifact schema. The actual port must preserve current
master's failure identity and replay behavior: the foundation predates master changes separating
top-level handler identity from the innermost reverter in `invariant/shrink.rs`, and master also
has a `skip_fresh_runs` path absent from this foundation. Reconcile those before adapting the
older campaign code. External SDK consumers and cross-version artifacts still require validation
when implementation changes the public boundary; no such validation is claimed here.
