//! Executable campaign requirements for the evm2 foundation, without a second campaign runner.
//!
//! The executor probes stage a copy-on-write successor and publish it only after inspecting the
//! result. The engine probes distinguish observation, acceptance, and backend export. These tests
//! use the workspace's pinned evm2 fork; they do not establish mainline or full Forge parity.

use alloy_consensus::{TxLegacy, transaction::Recovered};
use alloy_primitives::{Address, B256, Bytes, KECCAK256_EMPTY, U256, hex};
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
        constants::{CHEATCODE_ADDRESS, MAGIC_ASSUME},
        ethereum::{EthereumEnv, ForkState, LocalState, fork_db},
        opts::EvmOpts,
    },
    ethereum::{EthereumExecutor, EthereumFactory, EthereumInspectorStack},
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
    let mut session = campaign_executor(true);
    let original_nonce =
        Database::get_account(session.state_mut(), &CALLER).unwrap().unwrap().nonce;
    let mut candidate = session.clone();
    candidate.env_mut().block.number += U256::from(3);
    let result = candidate.transact_raw(CALLER, CONTRACT, Bytes::new(), U256::ZERO).unwrap();
    assert!(!result.status);
    assert_eq!(result.output.as_ref(), MAGIC_ASSUME);
    // EVM rollback does not roll back the host's warp or diagnostic recording.
    assert_eq!(candidate.env().block.timestamp, U256::from(999));
    assert_eq!(recorded_logs(&candidate), 1);
    assert_eq!(candidate.inspector_mut().take_logs().len(), 1);
    assert_eq!(slot(candidate.state_mut()), U256::from(7));
    assert_eq!(
        Database::get_account(candidate.state_mut(), &CALLER).unwrap().unwrap().nonce,
        original_nonce + 1,
    );
    drop(candidate);

    assert_eq!(slot(session.state_mut()), U256::from(7));
    assert_eq!(session.env().block.number, U256::ONE);
    assert_eq!(session.env().block.timestamp, U256::from(123));
    assert_eq!(recorded_logs(&session), 0);
    assert!(session.inspector_mut().take_logs().is_empty());
    assert_eq!(
        Database::get_account(session.state_mut(), &CALLER).unwrap().unwrap().nonce,
        original_nonce,
    );
}

#[test]
fn ordinary_revert_can_be_accepted_without_accepting_its_storage_writes() {
    let mut state = counter_state(true);
    state.database_mut().insert_account_info(
        &CONTRACT,
        AccountInfo::default().with_code(Bytecode::new_legacy(hex!("5f546001015f555f5ffd").into())),
    );
    let mut session = EthereumExecutor::new(environment(), state);
    let mut candidate = session.clone();
    let result = candidate.transact_raw(CALLER, CONTRACT, Bytes::new(), U256::ZERO).unwrap();
    assert!(!result.status);
    assert_ne!(result.output.as_ref(), MAGIC_ASSUME);
    // The campaign may retain this step even though execution reverted.
    session = candidate;
    assert_eq!(slot(session.state_mut()), U256::from(7));
    assert_eq!(Database::get_account(session.state_mut(), &CALLER).unwrap().unwrap().nonce, 1);
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

fn recorded_logs<D: Database + Clone + 'static>(
    executor: &EthereumExecutor<D, EthereumInspectorStack>,
) -> usize {
    let result = executor
        .call_raw(
            CALLER,
            CHEATCODE_ADDRESS,
            Vm::getRecordedLogsCall {}.abi_encode().into(),
            U256::ZERO,
        )
        .unwrap();
    assert!(result.status);
    Vm::getRecordedLogsCall::abi_decode_returns(&result.output).unwrap().len()
}

fn assert_sequence_isolation<D: Database + Clone + 'static>(
    baseline: EthereumExecutor<D, EthereumInspectorStack>,
) {
    for _ in 0..2 {
        let mut session = baseline.clone();
        for expected in 8..=10 {
            let mut candidate = session.clone();
            candidate.env_mut().block.number += U256::ONE;
            let result =
                candidate.transact_raw(CALLER, CONTRACT, Bytes::new(), U256::ZERO).unwrap();
            assert!(result.status);
            assert_eq!(slot(candidate.state_mut()), U256::from(expected));
            assert_eq!(candidate.env().block.timestamp, U256::from(999));
            assert_eq!(recorded_logs(&candidate), expected - 7);
            assert_eq!(slot(session.state_mut()), U256::from(expected - 1));

            // Publish state, environment, and retained cheatcodes together after classification.
            session = candidate;

            // A predicate sees accepted state, but its writes and inspector changes are discarded.
            let result = session.call_raw(CALLER, CONTRACT, Bytes::new(), U256::ZERO).unwrap();
            assert!(result.status);
            assert_eq!(slot(session.state_mut()), U256::from(expected));
            assert_eq!(recorded_logs(&session), expected - 7);
        }
        assert_eq!(session.env().block.number, U256::from(4));
    }
    assert_eq!(baseline.env().block.timestamp, U256::from(123));
    assert_eq!(recorded_logs(&baseline), 0);
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
