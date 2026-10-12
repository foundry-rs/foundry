//! Script deployment, setup and execution over native Ethereum state.

use crate::{ScriptResult, build::ScriptPredeployLibraries, execute::PreExecutionState};
use alloy_consensus::{
    Transaction, TxEip2930, TxEip4844, TxEip7702, TxLegacy, transaction::Recovered,
};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use alloy_rpc_types::state::AccountOverride;
use evm2::{EvmFeatures, ExecutionConfig, ethereum::TxEnvelope, evm::EmptyDB};
use evm2_inspectors::tracing::{TracingInspector, TracingInspectorConfig};
use eyre::{Result, ensure};
use foundry_cheatcodes::{BroadcastableTransaction, CheatsConfig, ethereum::Cheatcodes};
use foundry_cli::utils::needs_setup;
use foundry_common::{FoundryTransactionBuilder, LIBRARY_DEPLOYER, TransactionMaybeSigned};
use foundry_config::Config;
use foundry_evm::{
    core::{
        constants::{CALLER, CHEATCODE_ADDRESS, DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE},
        ethereum::{EthereumEnv, EthereumFork, FoundryContext},
        evm::{FoundryEvmNetwork, TransactionRequestFor},
    },
    ethereum::Executor,
    traces::{
        TraceKind,
        ethereum::{native_arena, render_status},
    },
};
use std::{fmt, mem, sync::Arc};

pub(crate) struct Simulation {
    executor: Executor<EmptyDB>,
    env: EthereumEnv,
}

impl fmt::Debug for Simulation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EthereumSimulation").field("env", &self.env).finish_non_exhaustive()
    }
}

impl Simulation {
    pub(crate) fn from_fork(
        mut fork: EthereumFork,
        config: &Config,
        opts: &foundry_evm::opts::EvmOpts,
    ) -> Result<Self> {
        let mut env = EthereumEnv::local_from_config(config, opts)?;
        env.block = fork.env.block;
        env.version.chain_id = fork.env.version.chain_id;
        env.gas_price = fork.env.gas_price;
        env.block.gas_limit = U256::from(opts.gas_limit());
        env.version.features.remove(
            EvmFeatures::FEE_CHARGE
                | EvmFeatures::BASE_FEE_CHECK
                | EvmFeatures::NONCE_CHECK
                | EvmFeatures::EIP3607,
        );
        fork.env = env;
        fork.database = fork.database.with_spec(env.spec);
        let executor = Executor::from_fork(
            &fork,
            ExecutionConfig::for_spec_and_version(env.spec, env.version),
        );
        Ok(Self { executor, env })
    }

    pub(crate) fn advance_block(&mut self) {
        self.env.block.number += U256::ONE;
        self.executor.set_block(self.env.block);
    }

    pub(crate) fn is_empty_code(&self, address: Address) -> Result<bool> {
        Ok(self.executor.code(address)?.is_empty())
    }

    pub(crate) fn set_sender_nonce(&mut self, sender: Address, nonce: u64) -> Result<()> {
        set_nonce(&mut self.executor, sender, nonce)
    }

    pub(crate) fn execute<N: alloy_network::Network>(
        &mut self,
        request: &TransactionMaybeSigned<N>,
        is_fixed_gas_limit: bool,
    ) -> Result<ScriptResult<N>>
    where
        N::TransactionRequest: FoundryTransactionBuilder<N>,
    {
        let from = request.from().ok_or_else(|| eyre::eyre!("simulation requires a sender"))?;
        let to = request.to();
        let input = request.input().cloned().unwrap_or_default();
        let value = request.value().unwrap_or_default();
        let authorization_list = request.authorization_list();
        let nonce = self.executor.account_info(from)?.map_or(0, |info| info.nonce);
        let limit = self
            .env
            .version
            .tx_gas_limit_cap
            .min(u64::try_from(self.env.block.gas_limit).unwrap_or(u64::MAX));
        let limit = if is_fixed_gas_limit {
            u64::try_from(request.gas().ok_or_else(|| eyre::eyre!("fixed gas limit missing"))?)?
        } else {
            limit
        };
        let access_list = match request {
            TransactionMaybeSigned::Signed { tx, .. } => tx.access_list().cloned(),
            TransactionMaybeSigned::Unsigned(tx) => tx.access_list().cloned(),
        }
        .unwrap_or_default();
        let tx = if let Some(hashes) = request.blob_versioned_hashes() {
            ensure!(authorization_list.is_none(), "blob simulation cannot include authorizations");
            TxEnvelope::Eip4844(
                TxEip4844 {
                    to: to.ok_or_else(|| eyre::eyre!("blob simulation requires a destination"))?,
                    chain_id: self.env.version.chain_id,
                    nonce,
                    gas_limit: limit,
                    input,
                    value,
                    blob_versioned_hashes: hashes.to_vec(),
                    access_list,
                    // Simulation executes the intent; publishing resolves the transaction's fees.
                    max_fee_per_blob_gas: self.env.block.blob_basefee.try_into()?,
                    ..Default::default()
                }
                .into(),
            )
        } else if let Some(authorization_list) = authorization_list {
            TxEnvelope::Eip7702(
                TxEip7702 {
                    to: to
                        .ok_or_else(|| eyre::eyre!("EIP-7702 simulation requires a destination"))?,
                    chain_id: self.env.version.chain_id,
                    nonce,
                    gas_limit: limit,
                    input,
                    value,
                    authorization_list,
                    access_list,
                    ..Default::default()
                }
                .into(),
            )
        } else if !access_list.is_empty() {
            TxEnvelope::Eip2930(TxEip2930 {
                chain_id: self.env.version.chain_id,
                to: to.map_or(TxKind::Create, TxKind::Call),
                nonce,
                gas_limit: limit,
                input,
                value,
                access_list,
                ..Default::default()
            })
        } else {
            TxEnvelope::Legacy(TxLegacy {
                to: to.map_or(TxKind::Create, TxKind::Call),
                nonce,
                gas_limit: limit,
                input,
                value,
                ..Default::default()
            })
        };
        let mut tx = Recovered::new_unchecked(tx, from);
        let mut gas_used = 0;
        if !is_fixed_gas_limit {
            let probe = self.executor.call(&tx)?;
            gas_used = probe.tx_gas_used();
            if to.is_some() && probe.status {
                let mut search = crate::gas_search::GasSearch::new(gas_used);
                while let Some(limit) = search.next_limit() {
                    match tx.inner_mut() {
                        TxEnvelope::Legacy(tx) => tx.gas_limit = limit,
                        TxEnvelope::Eip2930(tx) => tx.gas_limit = limit,
                        TxEnvelope::Eip7702(tx) => tx.gas_limit = limit,
                        TxEnvelope::Eip4844(tx) => tx.as_mut().gas_limit = limit,
                        _ => unreachable!("native Script simulation envelope"),
                    }
                    let result = self.executor.call(&tx)?;
                    search.record(
                        limit,
                        matches!(
                            result.stop,
                            evm2::interpreter::InstrStop::Revert
                                | evm2::interpreter::InstrStop::OutOfGas
                                | evm2::interpreter::InstrStop::OutOfFunds
                        ),
                    );
                }
                gas_used = search.gas_used();
                match tx.inner_mut() {
                    TxEnvelope::Legacy(tx) => tx.gas_limit = limit,
                    TxEnvelope::Eip2930(tx) => tx.gas_limit = limit,
                    TxEnvelope::Eip7702(tx) => tx.gas_limit = limit,
                    TxEnvelope::Eip4844(tx) => tx.as_mut().gas_limit = limit,
                    _ => unreachable!("native Script simulation envelope"),
                }
            }
        }
        let tracing = TracingInspector::new(
            TracingInspectorConfig::default_parity().set_record_inputs(true).set_record_logs(true),
        );
        let (result, tracing) = self.executor.inspect_transact(&tx, tracing);
        let result = result?;
        if is_fixed_gas_limit {
            gas_used = result.tx_gas_used();
        }
        Ok(ScriptResult {
            success: result.status,
            returned: result.output,
            gas_used,
            logs: result.logs,
            traces: vec![(TraceKind::Execution, native_arena(&tracing))],
            address: result.created_address,
            exit_reason: Some(render_status(result.stop)),
            ..Default::default()
        })
    }
}

pub(super) async fn execute<FEN: FoundryEvmNetwork>(
    state: &PreExecutionState<FEN>,
    restricted: bool,
) -> Result<ScriptResult<FEN::Network>> {
    ensure!(!state.args.debug, "native Ethereum Script debugger is not migrated yet");
    let config = &state.script_config.config;
    let opts = &state.script_config.evm_opts;
    let mut fork = if opts.fork_url.is_some() {
        Some(
            EthereumFork::from_fork(
                config,
                opts,
                state
                    .script_config
                    .backend
                    .fork()?
                    .ok_or_else(|| eyre::eyre!("script source is not forked"))?,
            )
            .await?,
        )
    } else {
        None
    };
    let mut env = EthereumEnv::local_from_config(config, opts)?;
    if let Some(fork) = &fork {
        env.block = fork.env.block;
        env.version.chain_id = fork.env.version.chain_id;
        env.gas_price = fork.env.gas_price;
    }
    env.block.gas_limit = U256::from(opts.gas_limit());
    env.version.features.remove(
        EvmFeatures::FEE_CHARGE
            | EvmFeatures::BASE_FEE_CHECK
            | EvmFeatures::NONCE_CHECK
            | EvmFeatures::EIP3607,
    );
    let execution = ExecutionConfig::for_spec_and_version(env.spec, env.version);
    let mut executor = if let Some(fork) = &mut fork {
        fork.env = env;
        fork.database = fork.database.clone().with_spec(env.spec);
        Executor::from_fork(fork, execution)
    } else {
        Executor::new(EmptyDB::default(), env.spec, execution, env.block)
    };
    executor.set_context(FoundryContext {
        gas_price_override: Some(env.gas_price),
        basefee_override: Some(env.block.basefee),
        ..Default::default()
    });
    let mut cheats_config = CheatsConfig::new(
        config,
        opts.clone(),
        Some(state.build_data.known_contracts.clone()),
        Some(state.build_data.build_data.target.clone()),
        false,
    );
    if restricted {
        cheats_config.blocked_cheatcodes =
            crate::library_deployments::rerun_unsafe_cheatcode_selectors();
    }
    let mut cheats = Cheatcodes::new(Arc::new(cheats_config));
    cheats.wallets = Some(state.script_wallets.clone());
    if let Some(fork) = &fork {
        cheats.initialize_fork(fork);
    }
    executor.apply_state_overrides(
        [(
            CHEATCODE_ADDRESS,
            AccountOverride { code: Some(Bytes::from_static(&[0])), ..Default::default() },
        )]
        .into_iter()
        .collect(),
    )?;
    if fork.is_none() {
        executor.apply_state_overrides(
            [(
                opts.create2_deployer,
                AccountOverride {
                    code: Some(Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE)),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
        )?;
    }
    if !state.args.broadcast && opts.sender == Config::DEFAULT_SENDER {
        executor.set_balance(opts.sender, U256::MAX)?;
    }
    executor.set_balance(CALLER, U256::MAX)?;
    set_nonce(&mut executor, opts.sender, state.script_config.sender_nonce)?;
    let mut output = ScriptResult::default();
    let (local, onchain, create2) = match &state.build_data.predeploy_libraries {
        ScriptPredeployLibraries::Default { local, onchain } => (local, onchain, None),
        ScriptPredeployLibraries::Create2 { local, onchain, salt } => (local, onchain, Some(*salt)),
    };
    let library_info = executor.account_info(LIBRARY_DEPLOYER)?.unwrap_or_default();
    executor.set_balance(LIBRARY_DEPLOYER, U256::MAX)?;
    set_nonce(&mut executor, LIBRARY_DEPLOYER, 0)?;
    for library in local {
        let result = stage::<FEN::Network>(
            &mut executor,
            &mut cheats,
            LIBRARY_DEPLOYER,
            TxKind::Create,
            library.bytecode.clone(),
            &env,
            TraceKind::Deployment,
            true,
        )?;
        ensure!(
            result.success && result.address == Some(library.address),
            "local library deployment failed"
        );
        output.logs.extend(result.logs);
        output.traces.extend(result.traces);
    }
    executor.set_balance(LIBRARY_DEPLOYER, library_info.balance)?;
    set_nonce(&mut executor, LIBRARY_DEPLOYER, library_info.nonce)?;
    for library in onchain {
        let (to, input) = match create2 {
            Some(salt) => {
                if !executor.code(library.address)?.is_empty() {
                    continue;
                }
                (
                    TxKind::Call(opts.create2_deployer),
                    Bytes::from([salt.as_slice(), library.bytecode.as_ref()].concat()),
                )
            }
            None => (TxKind::Create, library.bytecode.clone()),
        };
        let nonce = executor.account_info(opts.sender)?.map_or(0, |info| info.nonce);
        let result = stage::<FEN::Network>(
            &mut executor,
            &mut cheats,
            opts.sender,
            to,
            input.clone(),
            &env,
            TraceKind::Deployment,
            true,
        )?;
        ensure!(result.success, "on-chain library deployment failed");
        // Library deployments are real transactions, whereas the cheatcode inspector treats
        // root calls as synthetic script wrappers and restores their sender nonce.
        set_nonce(
            &mut executor,
            opts.sender,
            nonce.checked_add(1).ok_or_else(|| eyre::eyre!("library sender nonce overflow"))?,
        )?;
        output.logs.extend(result.logs);
        output.traces.extend(result.traces);
        let request = TransactionRequestFor::<FEN>::default()
            .with_from(opts.sender)
            .with_kind(to)
            .with_input(input)
            .with_nonce(nonce);
        output.transactions.get_or_insert_with(Default::default).push_back(
            BroadcastableTransaction {
                rpc: opts.fork_url.clone(),
                transaction: TransactionMaybeSigned::new(request),
                contract_address: None,
            },
        );
    }
    let caller_nonce = executor.account_info(CALLER)?.map_or(0, |info| info.nonce);
    let script_nonce = if opts.sender == CALLER { u64::MAX / 2 } else { caller_nonce };
    if opts.sender == CALLER {
        set_nonce(&mut executor, CALLER, script_nonce)?;
    }
    let address = CALLER.create(script_nonce);
    executor.set_balance(address, opts.initial_balance)?;
    let deployment = stage::<FEN::Network>(
        &mut executor,
        &mut cheats,
        CALLER,
        TxKind::Create,
        state.execution_data.bytecode.clone(),
        &env,
        TraceKind::Deployment,
        true,
    )?;
    ensure!(
        deployment.success,
        "native script deployment failed: {}",
        alloy_primitives::hex::encode(&deployment.returned)
    );
    output.logs.extend(deployment.logs);
    output.traces.extend(deployment.traces);
    if opts.sender == CALLER {
        set_nonce(&mut executor, CALLER, caller_nonce)?;
    }
    if config.script_execution_protection {
        cheats.script_address = Some(address);
    }
    if needs_setup(&state.execution_data.abi) {
        let setup = stage::<FEN::Network>(
            &mut executor,
            &mut cheats,
            opts.sender,
            TxKind::Call(address),
            Bytes::from_static(&[0x0a, 0x92, 0x54, 0xe4]),
            &env,
            TraceKind::Setup,
            true,
        )?;
        ensure!(
            setup.success,
            "native script setup failed: {}",
            alloy_primitives::hex::encode(&setup.returned)
        );
        output.logs.extend(setup.logs);
        output.traces.extend(setup.traces);
    }
    let result = stage::<FEN::Network>(
        &mut executor,
        &mut cheats,
        opts.sender,
        TxKind::Call(address),
        state.execution_data.calldata.clone(),
        &env,
        TraceKind::Execution,
        false,
    )?;
    output.success = result.success;
    output.returned = result.returned;
    output.gas_used = result.gas_used;
    output.exit_reason = result.exit_reason;
    output.logs.extend(result.logs);
    output.traces.extend(result.traces);
    let transactions = output.transactions.get_or_insert_with(Default::default);
    for transaction in cheats.transactions {
        let TransactionMaybeSigned::Unsigned(request) = transaction.transaction else {
            unreachable!("native broadcast collects unsigned requests")
        };
        let mut target = TransactionRequestFor::<FEN>::default()
            .with_from(request.from.expect("native sender"))
            .with_kind(request.to.unwrap_or(TxKind::Create))
            .with_input(request.input.into_input().unwrap_or_default())
            .with_value(request.value.unwrap_or_default())
            .with_nonce(request.nonce.expect("native nonce"));
        if let Some(chain_id) = request.chain_id {
            target.set_chain_id(chain_id);
        }
        if let Some(gas) = request.gas {
            target.set_gas_limit(gas);
        }
        if let Some(sidecar) = request.sidecar {
            target.set_blob_sidecar(sidecar);
        }
        if let Some(authorizations) = request.authorization_list {
            target.set_authorization_list(authorizations);
        }
        transactions.push_back(BroadcastableTransaction {
            rpc: transaction.rpc,
            transaction: TransactionMaybeSigned::new(target),
            contract_address: transaction.contract_address,
        });
    }
    Ok(output)
}

fn set_nonce(executor: &mut Executor<EmptyDB>, address: Address, nonce: u64) -> Result<()> {
    executor.apply_state_overrides(
        [(address, AccountOverride { nonce: Some(nonce), ..Default::default() })]
            .into_iter()
            .collect(),
    )
}

#[allow(clippy::too_many_arguments)]
fn stage<N: alloy_network::Network>(
    executor: &mut Executor<EmptyDB>,
    cheats: &mut Cheatcodes,
    from: Address,
    to: TxKind,
    input: Bytes,
    env: &EthereumEnv,
    kind: TraceKind,
    commit: bool,
) -> Result<ScriptResult<N>> {
    let nonce = executor.account_info(from)?.map_or(0, |info| info.nonce);
    let tx = Recovered::new_unchecked(
        TxEnvelope::Legacy(TxLegacy {
            to,
            nonce,
            input,
            gas_limit: env
                .version
                .tx_gas_limit_cap
                .min(u64::try_from(env.block.gas_limit).unwrap_or(u64::MAX)),
            ..Default::default()
        }),
        from,
    );
    cheats.tracing = Some(TracingInspector::new(
        TracingInspectorConfig::default_parity().set_record_inputs(true).set_record_logs(true),
    ));
    let (result, next) = if commit {
        executor.inspect_transact(&tx, mem::take(cheats))
    } else {
        executor.inspect_call(&tx, mem::take(cheats))
    };
    *cheats = next;
    let result = result?;
    let gas_used = result.tx_gas_used();
    Ok(ScriptResult {
        success: result.status,
        returned: result.output,
        gas_used,
        logs: mem::take(&mut cheats.logs),
        traces: vec![(kind, native_arena(cheats.tracing.as_ref().expect("script tracing")))],
        exit_reason: Some(render_status(result.stop)),
        address: result.created_address,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_consensus::SignableTransaction;
    use alloy_primitives::{B256, Signature, bytes};
    use alloy_rpc_types::TransactionRequest;
    use anvil::NodeConfig;
    use evm2::SpecId;
    use foundry_compilers::artifacts::EvmVersion;
    use foundry_evm::opts::EvmOpts;

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_simulation_uses_configured_spec_and_pinned_source_state() {
        let (api, handle) = anvil::spawn(NodeConfig::test()).await;
        let target = Address::with_last_byte(0xc0);
        // PUSH0 is valid on the source, but is unavailable to a London script simulation.
        api.anvil_set_code(target, bytes!("5f60005260206000f3")).await.unwrap();
        let mut opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            memory_limit: 128 * 1024 * 1024,
            ..Default::default()
        };
        opts.env.gas_limit = 1_000_000.into();
        let config = Config { evm_version: EvmVersion::London, ..Default::default() };
        let fork = EthereumFork::open(&config, &opts, None).await.unwrap();
        let mut runner = Simulation::from_fork(fork, &config, &opts).unwrap();
        assert_eq!(runner.env.spec, SpecId::LONDON);
        assert!(!runner.is_empty_code(target).unwrap());
        let result = runner
            .execute::<alloy_network::Ethereum>(
                &TransactionMaybeSigned::new(
                    TransactionRequest::default()
                        .with_from(Address::with_last_byte(0xaa))
                        .with_to(target),
                ),
                false,
            )
            .unwrap();
        assert!(!result.success);

        let config = Config { evm_version: EvmVersion::Shanghai, ..Default::default() };
        let fork = EthereumFork::open(&config, &opts, None).await.unwrap();
        let mut runner = Simulation::from_fork(fork, &config, &opts).unwrap();
        let result = runner
            .execute::<alloy_network::Ethereum>(
                &TransactionMaybeSigned::new(
                    TransactionRequest::default()
                        .with_from(Address::with_last_byte(0xaa))
                        .with_to(target),
                ),
                false,
            )
            .unwrap();
        assert!(result.success);
        assert_eq!(result.returned, Bytes::from(vec![0; 32]));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_simulation_preserves_unsigned_and_signed_blob_hashes() {
        let (api, handle) = anvil::spawn(NodeConfig::test()).await;
        let target = Address::with_last_byte(0xc0);
        api.anvil_set_code(target, bytes!("60004960005260206000f3")).await.unwrap();
        let mut opts = EvmOpts {
            fork_url: Some(handle.http_endpoint()),
            memory_limit: 128 * 1024 * 1024,
            ..Default::default()
        };
        opts.env.gas_limit = 1_000_000.into();
        let config = Config { evm_version: EvmVersion::Cancun, ..Default::default() };
        let sender = handle.dev_accounts().next().unwrap();
        let hash = B256::from([1; 32]);
        let mut request = TransactionRequest::default().with_from(sender).with_to(target);
        request.set_blob_versioned_hashes(vec![hash]);
        let unsigned = TransactionMaybeSigned::new(request);
        let signed = TransactionMaybeSigned::Signed {
            tx: TxEip4844 {
                chain_id: 31337,
                gas_limit: 1_000_000,
                to: target,
                blob_versioned_hashes: vec![hash],
                ..Default::default()
            }
            .into_signed(Signature::new(U256::ONE, U256::ONE, false))
            .into(),
            from: sender,
        };
        for request in [unsigned, signed] {
            let fork = EthereumFork::open(&config, &opts, None).await.unwrap();
            let mut runner = Simulation::from_fork(fork, &config, &opts).unwrap();
            let result = runner.execute::<alloy_network::Ethereum>(&request, false).unwrap();
            assert!(result.success);
            assert_eq!(result.returned.as_ref(), hash.as_slice());
        }
    }
    #[test]
    fn fixed_gas_simulation_preserves_block_limit_and_enforces_transaction_limit() {
        let mut opts = EvmOpts { memory_limit: 128 * 1024 * 1024, ..Default::default() };
        opts.env.gas_limit = 1_000_000.into();
        let env = EthereumEnv::local_from_config(&Config::default(), &opts).unwrap();
        let executor = Executor::new(
            EmptyDB::default(),
            env.spec,
            ExecutionConfig::for_spec_and_version(env.spec, env.version),
            env.block,
        );
        let mut runner = Simulation { executor, env };
        let sender = Address::with_last_byte(0xaa);
        let target = Address::with_last_byte(0xc0);
        runner
            .executor
            .apply_state_overrides(
                [(
                    target,
                    AccountOverride {
                        code: Some(bytes!("4560005260206000f3")),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
            )
            .unwrap();
        let request =
            TransactionRequest::default().with_from(sender).with_to(target).with_gas_limit(50_000);
        let result = runner
            .execute::<alloy_network::Ethereum>(&TransactionMaybeSigned::new(request.clone()), true)
            .unwrap();
        assert!(result.success);
        assert_eq!(U256::from_be_slice(&result.returned), env.block.gas_limit);
        assert_eq!(runner.executor.block().gas_limit, env.block.gas_limit);
        let result = runner
            .execute::<alloy_network::Ethereum>(
                &TransactionMaybeSigned::new(request.with_gas_limit(21_001)),
                true,
            )
            .unwrap();
        assert!(!result.success);
        assert_eq!(runner.executor.block().gas_limit, env.block.gas_limit);
    }
}
