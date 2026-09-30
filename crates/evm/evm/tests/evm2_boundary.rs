//! Executable campaign requirements for the evm2 foundation, without a second campaign runner.
//!
//! The session probes inspect a pending call and explicitly accept or discard its effects.
//! The engine probes distinguish observation, acceptance, and backend export. These tests
//! use the workspace's pinned evm2 fork; they do not establish mainline or full Forge parity.

use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::{Address, B256, Bytes, KECCAK256_EMPTY, TxKind, U256, hex};
use alloy_sol_types::SolCall;
use evm2::{
    SpecId,
    bytecode::Bytecode,
    env::BlockEnvExt,
    ethereum::TxEnvelope,
    evm::{
        AccountChangeRef, AccountInfo, Database, Db, EmptyDB, PendingState, StateChangeSink,
        StateChangeSource, StorageChange,
    },
};
use foundry_cheatcodes::{Vm, ethereum::CheatcodeAccessMode};
use foundry_evm::{
    core::{
        constants::{CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT, MAGIC_ASSUME},
        ethereum::{EthereumEnv, ForkState, LocalState, fork_db},
        opts::EvmOpts,
    },
    ethereum::{EthereumExecutor, EthereumFactory, EthereumInspectorStack},
    session::{CallRequest, CallStatus, ExecutionSession, StateObservation},
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tiny_http::{Response, Server};

const CALLER: Address = Address::with_last_byte(0xaa);
const CONTRACT: Address = Address::with_last_byte(0xbb);

#[test]
fn staged_handler_acceptance_and_predicate_discard_preserve_the_run_baseline() {
    assert_sequence_isolation(campaign_executor(false));
}

#[tokio::test(flavor = "multi_thread")]
async fn fork_sequence_acceptance_and_reset_preserve_remote_state() {
    let server = Server::http("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", server.server_addr());
    let rpc = std::thread::spawn(move || {
        // The target's code and slot zero are the only remote reads; other accounts are seeded.
        for _ in 0..2 {
            let mut request = server.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
            let body: serde_json::Value = serde_json::from_reader(request.as_reader()).unwrap();
            let result = match body["method"].as_str().unwrap() {
                "eth_getAccountInfo" => serde_json::json!({
                    "balance": "0x0", "nonce": "0x0", "code": handler_code(false),
                }),
                "eth_getStorageAt" => serde_json::json!("0x7"),
                method => panic!("unexpected RPC method: {method}"),
            };
            let response =
                serde_json::json!({"jsonrpc": "2.0", "id": body["id"], "result": result});
            request.respond(Response::from_string(response.to_string())).unwrap();
        }
    });
    let meta = fork_db::cache::BlockchainDbMeta::new(serde_json::Value::Null, endpoint.clone())
        .with_account_fetch_policy(fork_db::AccountFetchPolicy::RequireAccountInfo);
    let db = fork_db::BlockchainDb::new(meta, None);
    for address in [CALLER, Address::ZERO] {
        db.accounts().write().insert(address, AccountInfo::default());
    }
    let provider = EvmOpts::default().fork_provider_with_url(&endpoint).unwrap();
    let backend = fork_db::SharedBackend::spawn_backend(Arc::new(provider), db.clone(), None).await;
    let mut executor = recording_executor(ForkState::new(backend), CheatcodeAccessMode::Forked);
    executor.inspector_mut().cheatcodes_mut().allow_caller(CONTRACT);
    assert_sequence_isolation(executor);
    assert_eq!(db.storage().read()[&CONTRACT][&U256::ZERO], U256::from(7));
    rpc.join().unwrap();
}

#[test]
fn assumption_rejection_discards_delays_environment_and_retained_cheatcodes() {
    let mut session = ExecutionSession::from_ethereum(campaign_executor(true));
    let original_nonce = session.nonce(CALLER).unwrap();
    let mut request = CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO);
    request.block_delay = U256::from(3);
    let pending = session.execute(request).unwrap();
    assert!(pending.report().assumption_rejected());
    assert_eq!(pending.report().logs().len(), 1);
    let report = pending.discard();
    assert_eq!(report.output().as_ref(), MAGIC_ASSUME);
    assert_eq!(report.logs().len(), 1);
    assert_eq!(session.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(7));
    assert_eq!(session.block_number(), U256::ONE);
    assert_eq!(session.block_timestamp(), U256::from(123));
    assert_eq!(recorded_logs(&mut session), 0);
    assert_eq!(session.nonce(CALLER).unwrap(), original_nonce);
}

#[test]
fn ordinary_revert_can_be_accepted_without_accepting_its_storage_writes() {
    let mut state = counter_state(true);
    state.database_mut().insert_account_info(
        &CONTRACT,
        AccountInfo::default().with_code(Bytecode::new_legacy(hex!("5f546001015f555f5ffd").into())),
    );
    let mut session =
        ExecutionSession::from_ethereum(recording_executor(state, CheatcodeAccessMode::Local));
    let original_nonce = session.nonce(CALLER).unwrap();
    let pending = session
        .execute(CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO))
        .unwrap();
    assert_eq!(pending.report().status(), CallStatus::Revert);
    assert!(!pending.report().assumption_rejected());
    // The campaign may retain this step even though execution reverted.
    let _ = pending.accept();
    assert_eq!(session.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(7));
    assert_eq!(session.nonce(CALLER).unwrap(), original_nonce + 1);
}

#[test]
fn deployment_is_staged_and_can_be_repeated_after_discard() {
    let mut session = ExecutionSession::from_ethereum(campaign_executor(false));
    let nonce = session.nonce(CALLER).unwrap();
    let address = CALLER.create(nonce);
    // Initcode returning a one-byte STOP runtime.
    let request =
        CallRequest::new(CALLER, TxKind::Create, hex!("60005f5360015ff3").into(), U256::ZERO);
    let pending = session.execute(request.clone()).unwrap();
    assert_eq!(pending.report().status(), CallStatus::Success);
    assert_eq!(pending.report().created_address(), Some(address));
    let mut created = false;
    pending.visit_state(|observation| {
        if let StateObservation::Account(account) = observation
            && account.address == address
        {
            created = account.created;
        }
    });
    assert!(created);
    let _ = pending.discard();
    assert_eq!(session.nonce(address).unwrap(), 0);
    let report = session.execute(request).unwrap().accept();
    assert_eq!(report.created_address(), Some(address));
    assert_eq!(session.nonce(address).unwrap(), 1);
    assert_eq!(session.nonce(CALLER).unwrap(), nonce + 1);
}

#[test]
fn dropped_calls_and_execution_errors_leave_the_whole_session_unchanged() {
    let mut session = ExecutionSession::from_ethereum(campaign_executor(false));
    let nonce = session.nonce(CALLER).unwrap();
    let mut request = CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO);
    request.block_delay = U256::from(3);
    request.time_delay = U256::from(5);
    drop(session.execute(request.clone()).unwrap());
    request.gas_limit = Some(1);
    assert!(session.execute(request).is_err());
    assert_eq!(session.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(7));
    assert_eq!(session.nonce(CALLER).unwrap(), nonce);
    assert_eq!(session.block_number(), U256::ONE);
    assert_eq!(session.block_timestamp(), U256::from(123));
    assert_eq!(recorded_logs(&mut session), 0);
}

#[test]
fn shared_feedback_includes_unchanged_reads_and_resolves_backing_bytecode() {
    let executor = recording_executor(counter_state(false), CheatcodeAccessMode::Local);
    let mut session = ExecutionSession::from_ethereum(executor);
    let mut pending = session
        .execute(CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO))
        .unwrap();
    let mut reads = Vec::new();
    let mut code_hash = None;
    pending.visit_state(|observation| match observation {
        StateObservation::Storage(slot) if slot.address == CONTRACT => {
            reads.push((slot.key, slot.original, slot.current));
        }
        StateObservation::Account(account) if account.address == CONTRACT => {
            code_hash = account.code_hash;
        }
        _ => {}
    });
    assert_eq!(reads, [(U256::ZERO, U256::from(7), U256::from(7))]);
    assert_eq!(pending.bytecode(code_hash.unwrap()).unwrap().as_ref(), hex!("5f545f5260205ff3"));
    let report = pending.discard();
    assert_eq!(U256::from_be_slice(report.output()), U256::from(7));
}

#[test]
fn legacy_assertion_probe_sees_staged_state_without_publishing_its_own_writes() {
    // Every selector increments storage and returns it, including the synthetic failed() probe.
    let executor = recording_executor(counter_state(true), CheatcodeAccessMode::Local);
    let mut session = ExecutionSession::from_ethereum(executor);
    let mut pending = session
        .execute(CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO))
        .unwrap();
    let facts = pending.test_facts(CONTRACT, true).unwrap();
    assert!(facts.legacy_failure);
    assert!(!facts.global_failure);
    assert!(!facts.call_global_failure);
    let report = pending.accept();
    assert_eq!(U256::from_be_slice(report.output()), U256::from(8));
    assert_eq!(session.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(8));
}

#[test]
fn global_failure_facts_distinguish_this_call_from_preexisting_failure() {
    let mut session = ExecutionSession::from_ethereum(campaign_executor(false));
    let input = Vm::storeCall {
        target: CHEATCODE_ADDRESS,
        slot: GLOBAL_FAIL_SLOT.into(),
        value: U256::ONE.into(),
    }
    .abi_encode()
    .into();
    let mut pending = session
        .execute(CallRequest::new(CALLER, CHEATCODE_ADDRESS.into(), input, U256::ZERO))
        .unwrap();
    let facts = pending.test_facts(CONTRACT, false).unwrap();
    assert!(facts.global_failure);
    assert!(facts.call_global_failure);
    let _ = pending.accept();
    let mut pending = session
        .execute(CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO))
        .unwrap();
    let facts = pending.test_facts(CONTRACT, false).unwrap();
    assert!(facts.global_failure);
    assert!(!facts.call_global_failure);
    let _ = pending.discard();
}

#[test]
fn discarded_reports_retain_coverage_without_leaking_it_into_the_next_call() {
    let mut executor = campaign_executor(false);
    executor.inspector_mut().enable_line_coverage();
    let mut session = ExecutionSession::from_ethereum(executor);
    let mut report = session
        .execute(CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO))
        .unwrap()
        .discard();
    assert!(!report.take_coverage().unwrap().is_empty());
    assert_eq!(report.logs().len(), 1);
    let mut report = session
        .execute(CallRequest::new(CALLER, Address::ZERO.into(), Bytes::new(), U256::ZERO))
        .unwrap()
        .accept();
    // The collector may record an empty-code entry for the EOA call.
    assert!(report.take_coverage().unwrap().values().all(|map| map.bytecode().is_empty()));
    assert!(report.logs().is_empty());
    assert_eq!(recorded_logs(&mut session), 0);
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ReadFeedback {
    storage: Vec<(Address, U256, U256)>,
    contracts: Vec<(Address, B256)>,
}

impl StateChangeSink for ReadFeedback {
    type Error = Infallible;

    fn storage_read(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), Self::Error> {
        self.storage.push((address, key, value));
        Ok(())
    }

    fn account_read(
        &mut self,
        address: Address,
        info: Option<&AccountInfo>,
    ) -> Result<(), Self::Error> {
        if let Some(info) = info
            && !info.code_hash.is_zero()
            && info.code_hash != KECCAK256_EMPTY
        {
            self.contracts.push((address, info.code_hash));
        }
        Ok(())
    }
}

#[test]
fn discarded_and_detached_calls_export_loaded_but_unchanged_dictionary_values() {
    let mut backing = counter_state(false);
    let mut evm = EthereumFactory.create(environment(), Db::new(&mut backing));
    let tx = transaction();
    let mut discarded = ReadFeedback::default();
    let result = evm.transact(&tx).unwrap().discard_with(&mut discarded).unwrap();
    assert!(result.status);
    assert_eq!(U256::from_be_slice(&result.output), U256::from(7));
    assert_eq!(discarded.storage, [(CONTRACT, U256::ZERO, U256::from(7))]);
    let code_hash = evm.state_mut().account_info_untracked(&CONTRACT).unwrap().unwrap().code_hash;
    assert_eq!(discarded.contracts, [(CONTRACT, code_hash)]);
    // Account observations carry a hash; unchanged bytecode can live only in the backing cache.
    assert_eq!(
        evm.database_mut().get_code_by_hash(&code_hash).unwrap().original_bytes().as_ref(),
        &hex!("5f545f5260205ff3"),
    );

    let detached = evm.transact(&tx).unwrap().detach();
    let mut feedback = ReadFeedback::default();
    detached.pending_state.visit(&mut feedback).unwrap();
    assert_eq!(feedback, discarded);
    assert_eq!(
        evm.state_mut().account_info_untracked(&CALLER).unwrap().map_or(0, |info| info.nonce),
        0
    );
}

#[test]
fn detached_export_is_explicit_and_next_executor_reads_the_accepted_backend() {
    let mut accepted = counter_state(true);
    let baseline = accepted.clone();
    let outcome = {
        let mut evm = EthereumFactory.create(environment(), Db::new(&mut accepted));
        let outcome = evm.transact(&transaction()).unwrap().detach();
        assert!(outcome.result.status);
        assert_eq!(
            evm.state_mut().storage_slot_untracked(&CONTRACT, &U256::ZERO).unwrap(),
            U256::from(7)
        );
        outcome
    };
    assert_eq!(slot(&mut accepted), U256::from(7));
    accepted.commit(&outcome.pending_state);
    assert_eq!(slot(&mut accepted), U256::from(8));
    let mut executor = EthereumExecutor::new(environment(), accepted);
    let result = executor.transact_raw(CALLER, CONTRACT, Bytes::new(), U256::ZERO).unwrap();
    assert!(result.status);
    assert_eq!(slot(executor.state_mut()), U256::from(9));
    executor = EthereumExecutor::new(environment(), baseline);
    assert_eq!(slot(executor.state_mut()), U256::from(7));
}

struct FailingExport {
    staged: LocalState,
}

impl StateChangeSink for FailingExport {
    type Error = &'static str;

    fn storage(&mut self, change: StorageChange) -> Result<(), Self::Error> {
        let mut pending = PendingState::default();
        pending.insert_storage(change.address, change.key, change.original, change.current);
        self.staged.commit(&pending);
        Ok(())
    }

    fn account(&mut self, _change: AccountChangeRef<'_>) -> Result<(), Self::Error> {
        Err("export failed after applying storage")
    }
}

#[test]
fn failed_export_requires_staging_the_sink_to_avoid_partial_backend_publication() {
    let mut accepted = counter_state(true);
    let mut sink = FailingExport { staged: accepted.clone() };
    {
        let mut evm = EthereumFactory.create(environment(), Db::new(&mut accepted));
        let result = evm.transact(&transaction()).unwrap().commit_with(&mut sink);
        assert_eq!(result.unwrap_err(), "export failed after applying storage");
        assert_eq!(
            evm.state_mut().storage_slot_untracked(&CONTRACT, &U256::ZERO).unwrap(),
            U256::from(7)
        );
        assert_eq!(
            evm.state_mut().account_info_untracked(&CALLER).unwrap().map_or(0, |info| info.nonce),
            0
        );
    }
    // evm2 discarded its own writes, but cannot undo the sink's earlier side effects.
    assert_eq!(slot(&mut sink.staged), U256::from(8));
    drop(sink);
    assert_eq!(slot(&mut accepted), U256::from(7));
}

fn environment() -> EthereumEnv {
    EthereumEnv::new(
        SpecId::CANCUN,
        BlockEnvExt {
            number: U256::ONE,
            timestamp: U256::from(123),
            gas_limit: U256::from(1_000_000),
            ..Default::default()
        },
    )
}

fn counter_state(write: bool) -> LocalState {
    let mut state = LocalState::default();
    // Either return slot zero, or increment it before returning.
    let code = if write {
        hex!("5f546001015f555f545f5260205ff3").to_vec()
    } else {
        hex!("5f545f5260205ff3").to_vec()
    };
    state.database_mut().insert_account_info(
        &CONTRACT,
        AccountInfo::default().with_code(Bytecode::new_legacy(code.into())),
    );
    state.database_mut().insert_account_storage(&CONTRACT, &U256::ZERO, &U256::from(7));
    state
}

fn slot<D: Database + Clone + 'static>(state: &mut LocalState<D>) -> U256 {
    Database::get_storage(state, &CONTRACT, &U256::ZERO).unwrap()
}

fn transaction() -> Recovered<TxEnvelope> {
    Recovered::new_unchecked(
        TxEnvelope::Legacy(TxLegacy {
            to: CONTRACT.into(),
            gas_limit: 100_000,
            ..Default::default()
        }),
        CALLER,
    )
}

fn campaign_executor(reject: bool) -> EthereumExecutor<EmptyDB, EthereumInspectorStack> {
    let mut state = counter_state(true);
    state.database_mut().insert_account_info(
        &CONTRACT,
        AccountInfo::default().with_code(Bytecode::new_legacy(handler_code(reject))),
    );
    recording_executor(state, CheatcodeAccessMode::Local)
}

fn recording_executor<D: Database + Clone + 'static>(
    state: LocalState<D>,
    access_mode: CheatcodeAccessMode,
) -> EthereumExecutor<D, EthereumInspectorStack> {
    let mut executor =
        EthereumExecutor::new_foundry(environment(), state, Default::default(), access_mode);
    executor.inspector_mut().cheatcodes_mut().allow_caller(CALLER);
    let result = executor
        .transact_raw(
            CALLER,
            CHEATCODE_ADDRESS,
            Vm::recordLogsCall {}.abi_encode().into(),
            U256::ZERO,
        )
        .unwrap();
    assert!(result.status);
    executor
}

fn recorded_logs<D: Database + Clone + 'static>(session: &mut ExecutionSession<D>) -> usize {
    let result = session
        .execute(CallRequest::new(
            CALLER,
            CHEATCODE_ADDRESS.into(),
            Vm::getRecordedLogsCall {}.abi_encode().into(),
            U256::ZERO,
        ))
        .unwrap()
        .discard();
    assert_eq!(result.status(), CallStatus::Success);
    Vm::getRecordedLogsCall::abi_decode_returns(result.output()).unwrap().len()
}

fn assert_sequence_isolation<D: Database + Clone + 'static>(
    baseline: EthereumExecutor<D, EthereumInspectorStack>,
) {
    let mut baseline = ExecutionSession::from_ethereum(baseline);
    let checkpoint = baseline.checkpoint();
    let mut session = checkpoint.spawn();
    for _ in 0..2 {
        session.restore(&checkpoint);
        for expected in 8..=10 {
            let mut request = CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO);
            request.block_delay = U256::ONE;
            let pending = session.execute(request).unwrap();
            assert_eq!(pending.report().status(), CallStatus::Success);
            let mut observed = None;
            pending.visit_state(|observation| {
                if let StateObservation::Storage(slot) = observation
                    && slot.address == CONTRACT
                    && slot.key == U256::ZERO
                {
                    observed = Some(slot.current);
                }
            });
            assert_eq!(observed, Some(U256::from(expected)));
            let report = pending.accept();
            assert_eq!(report.logs().len(), 1);
            assert_eq!(session.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(expected));
            assert_eq!(session.block_timestamp(), U256::from(999));
            assert_eq!(recorded_logs(&mut session), expected - 7);

            // A predicate sees accepted state, but its writes and inspector changes are discarded.
            let result = session
                .execute(CallRequest::new(CALLER, CONTRACT.into(), Bytes::new(), U256::ZERO))
                .unwrap()
                .discard();
            assert_eq!(result.status(), CallStatus::Success);
            assert_eq!(result.logs().len(), 1);
            assert_eq!(session.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(expected));
            assert_eq!(recorded_logs(&mut session), expected - 7);
        }
        assert_eq!(session.block_number(), U256::from(4));
    }
    assert_eq!(baseline.block_timestamp(), U256::from(123));
    assert_eq!(baseline.storage(CONTRACT, U256::ZERO).unwrap(), U256::from(7));
    assert_eq!(recorded_logs(&mut baseline), 0);
}

fn handler_code(reject: bool) -> Bytes {
    // Increment slot zero and emit an empty log, then warp and optionally reject the case.
    let mut code = hex!("5f546001015f555f5fa0").to_vec();
    let mut calls = vec![Vm::warpCall { newTimestamp: U256::from(999) }.abi_encode()];
    if reject {
        calls.push(Vm::assumeCall { condition: false }.abi_encode());
    }
    let mut offsets = Vec::new();
    for input in &calls {
        let length = u16::try_from(input.len()).unwrap().to_be_bytes();
        code.push(0x61); // PUSH2 length.
        code.extend_from_slice(&length);
        code.push(0x61); // PUSH2 payload offset, patched below.
        offsets.push(code.len());
        code.extend_from_slice(&[0, 0, 0x5f, 0x39, 0x5f, 0x5f, 0x61]);
        code.extend_from_slice(&length);
        code.extend_from_slice(&[0x5f, 0x5f, 0x73]);
        code.extend_from_slice(CHEATCODE_ADDRESS.as_slice());
        code.extend_from_slice(&[0x5a, 0xf1, 0x50]); // GAS, CALL, POP.
    }
    if reject {
        code.extend_from_slice(&hex!("3d5f5f3e3d5ffd")); // Bubble the assumption rejection.
    } else {
        code.push(0x00);
    }
    for (offset, input) in offsets.into_iter().zip(calls) {
        let start = u16::try_from(code.len()).unwrap().to_be_bytes();
        code[offset..offset + 2].copy_from_slice(&start);
        code.extend_from_slice(&input);
    }
    code.into()
}
