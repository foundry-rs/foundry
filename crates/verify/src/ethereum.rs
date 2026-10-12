//! Ethereum bytecode verification through EVM2.

use crate::bytecode::{RuntimeDeployment, RuntimeVerification, VerifyBytecodeArgs};
use alloy_consensus::{EthereumTypedTransaction, Transaction, TxLegacy, transaction::Recovered};
use alloy_network::{
    AnyRpcBlock, AnyRpcTransaction, AnyTxEnvelope, BlockResponse, TransactionResponse,
};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256};
use alloy_rpc_types::{BlockId, BlockTransactions, state::AccountOverride};
use evm2::{ExecutionConfig, ethereum::TxEnvelope, evm::DynDatabase};
use evm2_inspectors::tracing::{TracingInspector, TracingInspectorConfig};
use eyre::{Result, WrapErr, ensure};
use foundry_common::{SYSTEM_TRANSACTION_TYPE, is_known_system_sender};
use foundry_evm::{
    constants::{DEFAULT_CREATE2_DEPLOYER, DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE},
    core::{decode::RevertDecoder, ethereum::EthereumFork},
    ethereum::Executor,
};

impl RuntimeVerification {
    pub(super) async fn execute_ethereum(self) -> Result<()> {
        let (config, mut opts) = crate::utils::load_fork_config_and_evm_opts(&self.config)?;
        VerifyBytecodeArgs::apply_endpoint_expectation(
            &mut opts,
            self.endpoint_identity.as_ref(),
            self.network_was_inferred,
        );
        let fork = EthereumFork::open(&config, &opts, Some(BlockId::number(self.simulation_block)))
            .await?;
        VerifyBytecodeArgs::ensure_endpoint_identity_unchanged(
            &self.config,
            self.endpoint_identity.as_ref(),
        )
        .await?;
        if let Some(block) = &self.block {
            ensure!(
                block.header().hash == fork.fork.hash(),
                "verification block changed while verify-bytecode was running; retry against a stable endpoint"
            );
        }
        let database = match &self.deployment {
            RuntimeDeployment::Synthetic { .. } => fork.database.clone(),
            RuntimeDeployment::Creation(_) => fork.parent_database().await?,
        };
        let env = fork.env;
        let mut executor = Executor::new(
            database,
            env.spec,
            ExecutionConfig::for_spec_and_version(env.spec, env.version),
            env.block,
        );
        let (tx, onchain_block) = match &self.deployment {
            RuntimeDeployment::Synthetic { predeploy } => {
                let deployer = Address::with_last_byte(1);
                executor.apply_state_overrides(
                    [(
                        deployer,
                        AccountOverride {
                            balance: Some(U256::from(100 * 10_u128.pow(18))),
                            nonce: Some(0),
                            code: Some(Bytes::new()),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                )?;
                (
                    Recovered::new_unchecked(
                        TxEnvelope::Legacy(TxLegacy {
                            chain_id: Some(env.version.chain_id),
                            nonce: 0,
                            gas_price: env.block.basefee.try_into()?,
                            gas_limit: env.block.gas_limit.try_into()?,
                            to: TxKind::Create,
                            input: self.local_bytecode_vec.clone().into(),
                            ..Default::default()
                        }),
                        deployer,
                    ),
                    (!predeploy).then_some(self.simulation_block),
                )
            }
            RuntimeDeployment::Creation(transaction) => {
                replay_prefix(self.block.as_ref(), transaction.tx_hash(), &mut executor)?;
                let input = match transaction.kind() {
                    TxKind::Create => Bytes::from(self.local_bytecode_vec.clone()),
                    TxKind::Call(to) => {
                        ensure!(
                            to == DEFAULT_CREATE2_DEPLOYER,
                            "creation transaction uses a custom factory"
                        );
                        let salt = transaction.input().get(..32).ok_or_else(|| {
                            eyre::eyre!("CREATE2 transaction input is missing its salt")
                        })?;
                        let mut input = salt.to_vec();
                        input.extend_from_slice(&self.local_bytecode_vec);
                        if executor.code(to)?.is_empty() {
                            executor.apply_state_overrides(
                                [(
                                    to,
                                    AccountOverride {
                                        code: Some(Bytes::from_static(
                                            DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE,
                                        )),
                                        nonce: Some(1),
                                        ..Default::default()
                                    },
                                )]
                                .into_iter()
                                .collect(),
                            )?;
                        }
                        input.into()
                    }
                };
                (transaction_with_input(transaction, Some(input))?, Some(self.simulation_block))
            }
        };
        let address = deploy(&mut executor, &tx)?;
        let code = executor.code(address)?;
        self.report(code.original_byte_slice(), onchain_block).await
    }
}

fn transaction_with_input(
    tx: &AnyRpcTransaction,
    input: Option<Bytes>,
) -> Result<Recovered<TxEnvelope>> {
    let AnyTxEnvelope::Ethereum(envelope) = tx.inner.inner.inner() else {
        eyre::bail!(
            "cannot execute unknown transaction type {:#x} with EVM2",
            tx.transaction_type().unwrap_or_default()
        );
    };
    let typed =
        envelope.clone().map_eip4844(alloy_consensus::TxEip4844::from).into_typed_transaction();
    let tx_native = match typed {
        EthereumTypedTransaction::Legacy(mut tx) => {
            if let Some(input) = input {
                tx.input = input;
            }
            TxEnvelope::Legacy(tx)
        }
        EthereumTypedTransaction::Eip2930(mut tx) => {
            if let Some(input) = input {
                tx.input = input;
            }
            TxEnvelope::Eip2930(tx)
        }
        EthereumTypedTransaction::Eip1559(mut tx) => {
            if let Some(input) = input {
                tx.input = input;
            }
            TxEnvelope::Eip1559(tx)
        }
        EthereumTypedTransaction::Eip4844(mut tx) => {
            if let Some(input) = input {
                tx.input = input;
            }
            TxEnvelope::Eip4844(tx.into())
        }
        EthereumTypedTransaction::Eip7702(mut tx) => {
            if let Some(input) = input {
                tx.input = input;
            }
            TxEnvelope::Eip7702(tx.into())
        }
    };
    Ok(Recovered::new_unchecked(tx_native, tx.from()))
}

fn replay_prefix<D: DynDatabase + Clone + 'static>(
    block: Option<&AnyRpcBlock>,
    target: B256,
    executor: &mut Executor<D>,
) -> Result<()> {
    let block = block.ok_or_else(|| {
        eyre::eyre!("creation block is required to reconstruct transaction state")
    })?;
    let BlockTransactions::Full(txs) = block.transactions() else {
        eyre::bail!("Could not get block txs");
    };
    ensure!(
        txs.iter().any(|tx| tx.tx_hash() == target),
        "transaction {target:?} is missing from its block"
    );
    for tx in txs {
        if tx.tx_hash() == target {
            break;
        }
        if is_known_system_sender(tx.from())
            || tx.transaction_type() == Some(SYSTEM_TRANSACTION_TYPE)
        {
            continue;
        }
        let _ = executor
            .transact(&transaction_with_input(tx, None)?)
            .wrap_err_with(|| format!("Failed to execute transaction: {:?}", tx.tx_hash()))?;
    }
    Ok(())
}

fn deploy<D: DynDatabase + Clone + 'static>(
    executor: &mut Executor<D>,
    tx: &Recovered<TxEnvelope>,
) -> Result<Address> {
    let address = match tx.kind() {
        TxKind::Create => {
            tx.signer().create(executor.account_info(tx.signer())?.map_or(0, |info| info.nonce))
        }
        TxKind::Call(to) => {
            ensure!(to == DEFAULT_CREATE2_DEPLOYER, "creation transaction uses a custom factory");
            let salt = tx
                .input()
                .get(..32)
                .ok_or_else(|| eyre::eyre!("CREATE2 transaction input is missing its salt"))?;
            to.create2_from_code(B256::from_slice(salt), &tx.input()[32..])
        }
    };
    let inspector = TracingInspector::new(TracingInspectorConfig::default_parity());
    let (result, inspector) = executor.inspect_transact(tx, inspector);
    let result = result?;
    if !result.status {
        let reason = RevertDecoder::new().decode_data(&result.output);
        eyre::bail!(
            "Failed to deploy contract on fork at block: {reason}.\n\
            This typically happens when your local bytecode differs from what was actually deployed.\n\
            Common causes:\n\
            - Your contract source is not at the same commit used during deployment\n\
            - Cached build artifacts are stale (try `forge clean && forge build`)\n\
            - Compiler settings (optimizer, evm_version, via_ir) don't match the deployment"
        );
    }
    ensure!(
        inspector.traces().nodes().iter().any(|node| {
            node.trace.success && node.trace.kind.is_any_create() && node.trace.address == address
        }),
        "Transaction did not deploy a contract at the expected address {address}"
    );
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::{AnyHeader, AnyRpcHeader};
    use evm2::{SpecId, env::BlockEnvExt, evm::EmptyDB};

    const CALLER: Address = Address::with_last_byte(0x42);
    const INIT: &[u8] = &[0x60, 1, 0x60, 12, 0x60, 0, 0x39, 0x60, 1, 0x60, 0, 0xf3, 0];

    fn executor() -> Executor<EmptyDB> {
        let mut executor = Executor::new(
            EmptyDB::default(),
            SpecId::CANCUN,
            ExecutionConfig::for_spec_and_version(
                SpecId::CANCUN,
                evm2::Version::new(SpecId::CANCUN),
            ),
            BlockEnvExt { gas_limit: U256::from(30_000_000), ..Default::default() },
        );
        executor.set_balance(CALLER, U256::from(1_000_000_000)).unwrap();
        executor
    }

    fn transaction(nonce: u64, hash: B256, to: Option<Address>, input: Bytes) -> AnyRpcTransaction {
        serde_json::from_value(serde_json::json!({
            "type": "0x0", "hash": hash, "nonce": format!("0x{nonce:x}"),
            "from": CALLER, "to": to, "value": "0x0", "gas": "0xf4240",
            "gasPrice": "0x2", "input": input, "v": "0x1b", "r": "0x1", "s": "0x1"
        }))
        .unwrap()
    }

    fn block(txs: Vec<AnyRpcTransaction>) -> AnyRpcBlock {
        AnyRpcBlock::new(
            alloy_rpc_types::Block::new(
                AnyRpcHeader::from_sealed(AnyHeader::default().seal(B256::ZERO)),
                BlockTransactions::Full(txs),
            )
            .into(),
        )
    }

    #[test]
    fn replay_reverted_prefix_commits_nonce_and_fees_before_local_creation() {
        let mut executor = executor();
        let reverting = Address::with_last_byte(0x43);
        executor
            .apply_state_overrides(
                [(
                    reverting,
                    AccountOverride {
                        code: Some(Bytes::from_static(&[0x60, 0, 0x60, 0, 0xfd])),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
            )
            .unwrap();
        let target = transaction(1, B256::with_last_byte(2), None, Bytes::new());
        let block = block(vec![
            transaction(0, B256::with_last_byte(1), Some(reverting), Bytes::new()),
            target.clone(),
            transaction(2, B256::with_last_byte(3), None, Bytes::new()),
        ]);
        replay_prefix(Some(&block), target.tx_hash(), &mut executor).unwrap();
        let info = executor.account_info(CALLER).unwrap().unwrap();
        assert_eq!(info.nonce, 1);
        assert!(info.balance < U256::from(1_000_000_000));
        let tx = transaction_with_input(&target, Some(Bytes::from_static(INIT))).unwrap();
        assert_eq!(tx.gas_price(), Some(2));
        assert_eq!(tx.nonce(), 1);
        let address = deploy(&mut executor, &tx).unwrap();
        assert_eq!(address, CALLER.create(1));
        assert_eq!(executor.code(address).unwrap().original_bytes(), Bytes::from_static(&[0]));
    }

    #[test]
    fn missing_target_does_not_execute_prefix() {
        let mut executor = executor();
        let block = block(vec![transaction(0, B256::ZERO, None, Bytes::from_static(INIT))]);
        let error =
            replay_prefix(Some(&block), B256::with_last_byte(1), &mut executor).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("transaction {:?} is missing from its block", B256::with_last_byte(1))
        );
        assert_eq!(executor.account_info(CALLER).unwrap().unwrap().nonce, 0);
        let error = replay_prefix(None, B256::ZERO, &mut executor).unwrap_err();
        assert_eq!(
            error.to_string(),
            "creation block is required to reconstruct transaction state"
        );
        assert_eq!(executor.account_info(CALLER).unwrap().unwrap().nonce, 0);
    }

    #[test]
    fn create2_requires_a_real_deployment_and_reads_accepted_code() {
        let mut executor = executor();
        executor
            .apply_state_overrides(
                [(
                    DEFAULT_CREATE2_DEPLOYER,
                    AccountOverride {
                        code: Some(Bytes::from_static(DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE)),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
            )
            .unwrap();
        let mut input = B256::with_last_byte(7).to_vec();
        input.extend_from_slice(INIT);
        let tx = transaction(0, B256::ZERO, Some(DEFAULT_CREATE2_DEPLOYER), input.into());
        let tx = transaction_with_input(&tx, None).unwrap();
        let address = deploy(&mut executor, &tx).unwrap();
        assert_eq!(
            address,
            DEFAULT_CREATE2_DEPLOYER.create2_from_code(B256::with_last_byte(7), INIT)
        );
        assert_eq!(executor.code(address).unwrap().original_bytes(), Bytes::from_static(&[0]));
        // Repeating the same salt cannot mistake the existing code for a fresh creation.
        let (_, caller) = tx.clone().into_parts();
        let mut second = tx.inner().as_legacy().unwrap().clone();
        second.nonce = 1;
        assert!(
            deploy(&mut executor, &Recovered::new_unchecked(TxEnvelope::Legacy(second), caller))
                .is_err()
        );
    }
}
