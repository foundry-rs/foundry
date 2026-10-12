//! Read-only Ethereum RPC state for native execution.

use super::EthereumEnv;
use crate::{fork::Fork, opts::EvmOpts};
use alloy_consensus::{BlockHeader, transaction::Recovered};
use alloy_eips::{BlockId, BlockNumHash, eip7840::BlobParams};
use alloy_network::{AnyNetwork, AnyTxEnvelope, BlockResponse, TransactionResponse};
use alloy_primitives::{Address, B256, KECCAK256_EMPTY, U256, map::B256HashMap};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types::BlockTransactions;
use evm2::{
    DatabaseError, SpecId,
    bytecode::Bytecode,
    env::BlockEnvExt,
    ethereum::TxEnvelope,
    evm::{AccountInfo, DbResult, DynDatabase},
};
use eyre::{OptionExt, ensure, eyre};
use foundry_common::block_on_handle;
use foundry_config::Config;
use foundry_evm_hardforks::{EthereumHardfork, FoundryHardfork};
use parking_lot::Mutex;
use std::{collections::BTreeMap, sync::Arc};
use tokio::runtime::Handle;

/// An exact RPC state anchor and its native execution environment.
pub struct EthereumFork {
    pub env: EthereumEnv,
    pub database: RpcDatabase,
    pub fork: Fork,
    pub hardfork: Option<FoundryHardfork>,
}

impl EthereumFork {
    /// Returns the exact source block whose header defines this execution environment.
    pub fn source_block(&self) -> &alloy_network::AnyRpcBlock {
        &self.fork.block
    }

    /// Resolves one source identity before any native state reads.
    pub async fn open(
        config: &Config,
        opts: &EvmOpts,
        block: Option<BlockId>,
    ) -> eyre::Result<Self> {
        let mut fork =
            opts.prepare_fork().await?.ok_or_eyre("RPC execution requires a fork URL")?;
        let provider = fork.provider::<AnyNetwork>();
        opts.check_fork_endpoint(&provider, &fork).await?;
        if let Some(block) = block {
            ensure!(!block.is_pending(), "native execution requires a mined block");
            let response =
                provider.get_block(block).await?.ok_or_eyre("fork block is unavailable")?;
            fork = fork
                .at_block(BlockNumHash::new(response.header.number(), response.header.hash))
                .await?;
        }
        Self::from_fork(config, opts, fork).await
    }

    /// Builds native state from the exact source already used by preflight.
    pub async fn from_fork(config: &Config, opts: &EvmOpts, fork: Fork) -> eyre::Result<Self> {
        let provider = fork.provider::<AnyNetwork>();
        opts.check_fork_endpoint(&provider, &fork).await?;
        let context = fork.context();
        let header = &fork.block.header;
        let hardfork = config.hardfork.or(context.hardfork).or_else(|| {
            FoundryHardfork::from_chain_and_timestamp(context.source_chain_id, header.timestamp())
        });
        let mut config = config.clone();
        config.hardfork = hardfork;
        let mut env = EthereumEnv::local_from_config(&config, opts)?;
        env.version.chain_id = opts.chain_id_override().unwrap_or(context.execution_chain_id);
        env.gas_price = U256::from(match opts.env.gas_price {
            Some(price) => u128::from(price),
            None => provider.get_gas_price().await?,
        });
        let blob_params = if hardfork == Some(FoundryHardfork::Ethereum(EthereumHardfork::Bpo1)) {
            BlobParams::bpo1()
        } else if hardfork == Some(FoundryHardfork::Ethereum(EthereumHardfork::Bpo2)) {
            BlobParams::bpo2()
        } else if env.spec >= SpecId::OSAKA {
            BlobParams::osaka()
        } else if env.spec >= SpecId::PRAGUE {
            BlobParams::prague()
        } else {
            BlobParams::cancun()
        };
        env.block = BlockEnvExt {
            number: U256::from(header.number()),
            beneficiary: header.beneficiary(),
            timestamp: U256::from(header.timestamp()),
            gas_limit: U256::from(header.gas_limit()),
            basefee: U256::from(header.base_fee_per_gas().unwrap_or_default()),
            difficulty: header.difficulty(),
            prevrandao: U256::from_be_slice(header.mix_hash().unwrap_or_default().as_slice()),
            blob_basefee: U256::from(BlockHeader::blob_fee(header, blob_params).unwrap_or(1)),
            slot_num: U256::from(header.slot_number().unwrap_or_default()),
            ..Default::default()
        };
        let database = RpcDatabase::new(&fork, env.spec);
        Ok(Self { env, database, fork, hardfork })
    }

    /// Reads the parent state while retaining this block's execution environment for replay.
    pub async fn parent_database(&self) -> eyre::Result<RpcDatabase> {
        let header = &self.fork.block.header;
        ensure!(header.number() > 0, "genesis has no parent state");
        let parent = self
            .fork
            .at_block(BlockNumHash::new(header.number() - 1, header.parent_hash()))
            .await?;
        Ok(RpcDatabase::new(&parent, self.env.spec))
    }

    /// Pins a transaction's block and returns its prefix and target over parent state.
    pub async fn transaction_position(
        config: &Config,
        opts: &EvmOpts,
        hash: B256,
    ) -> eyre::Result<(Self, Vec<Recovered<TxEnvelope>>, Recovered<TxEnvelope>)> {
        ensure!(!opts.fork_state_by_number, "transaction replay requires hash-addressed state");
        let source = Self::open(config, opts, None).await?;
        let provider = source.fork.provider::<AnyNetwork>();
        let target =
            provider.get_transaction_by_hash(hash).await?.ok_or_eyre("transaction not found")?;
        let block_hash = target.block_hash().ok_or_eyre("transaction is still pending")?;
        let block = provider
            .get_block(BlockId::hash(block_hash))
            .full()
            .await?
            .ok_or_eyre("transaction block is unavailable")?;
        ensure!(block.header.number() > 0, "cannot replay a transaction in genesis");
        ensure!(block.header.hash == block_hash, "transaction block changed");
        let BlockTransactions::Full(transactions) = block.transactions() else {
            eyre::bail!("transaction block does not include full transactions");
        };
        let index = transactions
            .iter()
            .position(|tx| tx.tx_hash() == hash)
            .ok_or_eyre("transaction is missing from its block")?;
        ensure!(target.transaction_index() == Some(index as u64), "transaction index changed");
        let mut prefix = Vec::with_capacity(index + 1);
        for tx in &transactions[..=index] {
            let AnyTxEnvelope::Ethereum(envelope) = tx.inner.inner.inner() else {
                eyre::bail!("unsupported transaction type in Ethereum fork prefix");
            };
            prefix.push(Recovered::new_unchecked(
                envelope.clone().map_eip4844(alloy_consensus::TxEip4844::from).into(),
                tx.from(),
            ));
        }
        let fork =
            source.fork.at_block(BlockNumHash::new(block.header.number(), block_hash)).await?;
        let mut fork = Self::from_fork(config, opts, fork).await?;
        fork.database = fork.parent_database().await?;
        let target = prefix.pop().expect("target transaction included");
        Ok((fork, prefix, target))
    }
}

/// Read-only remote state. Accepted writes belong exclusively to the executor's state.
///
/// Clones share only content-addressed code and anchored block ancestry, never writable accounts
/// or storage. Calls must run on a blocking worker or a multithreaded Tokio runtime.
#[derive(Clone)]
pub struct RpcDatabase {
    provider: RootProvider<AnyNetwork>,
    block: BlockId,
    spec: SpecId,
    handle: Handle,
    code: Arc<Mutex<B256HashMap<Bytecode>>>,
    ancestry: Arc<Mutex<BTreeMap<u64, B256>>>,
}

impl RpcDatabase {
    fn new(fork: &Fork, spec: SpecId) -> Self {
        let header = &fork.block.header;
        let mut ancestry = BTreeMap::from([(header.number(), header.hash)]);
        if header.number() > 0 {
            ancestry.insert(header.number() - 1, header.parent_hash());
        }
        Self {
            provider: fork.provider(),
            block: fork.state_block_id(),
            spec,
            handle: Handle::current(),
            code: Arc::default(),
            ancestry: Arc::new(Mutex::new(ancestry)),
        }
    }

    fn error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> DatabaseError {
        DatabaseError::new(std::io::Error::other(error.into()), true)
    }

    /// Applies the execution hardfork's account-presence policy to the pinned source state.
    pub const fn with_spec(mut self, spec: SpecId) -> Self {
        self.spec = spec;
        self
    }
}

impl DynDatabase for RpcDatabase {
    fn get_account(&mut self, address: &Address) -> DbResult<Option<AccountInfo>> {
        let (balance, nonce, code) = block_on_handle(&self.handle, async {
            tokio::try_join!(
                self.provider.get_balance(*address).block_id(self.block).into_future(),
                self.provider.get_transaction_count(*address).block_id(self.block).into_future(),
                self.provider.get_code_at(*address).block_id(self.block).into_future(),
            )
        })
        .map_err(|error| {
            Self::error(
                eyre::Report::new(error).wrap_err(format!("failed to get account for {address}")),
            )
        })?;
        let code = Bytecode::new_raw_checked(code).map_err(Self::error)?;
        let info =
            AccountInfo::default().with_balance(balance).with_nonce(nonce).with_code(code.clone());
        self.code.lock().insert(info.code_hash, code);
        // Match Foundry's empty-account normalization. RPC reads return zero fields for missing
        // accounts; reporting those as present would grant a fresh EIP-7702 authority a refund.
        Ok((self.spec < SpecId::SPURIOUS_DRAGON || !info.is_empty()).then_some(info))
    }

    fn get_code_by_hash(&mut self, hash: &B256) -> DbResult<Bytecode> {
        if *hash == KECCAK256_EMPTY || hash.is_zero() {
            return Ok(Bytecode::default());
        }
        self.code
            .lock()
            .get(hash)
            .cloned()
            .ok_or_else(|| Self::error(eyre!("RPC bytecode was not loaded for hash {hash}")))
    }

    fn get_storage(&mut self, address: &Address, slot: &U256) -> DbResult<U256> {
        block_on_handle(
            &self.handle,
            self.provider.get_storage_at(*address, *slot).block_id(self.block).into_future(),
        )
        .map_err(Self::error)
    }

    fn get_block_hash(&mut self, number: &U256) -> DbResult<B256> {
        let number = u64::try_from(*number).map_err(Self::error)?;
        loop {
            let (height, hash) = {
                let ancestry = self.ancestry.lock();
                if let Some(hash) = ancestry.get(&number) {
                    return Ok(*hash);
                }
                let (&height, &hash) =
                    ancestry.first_key_value().expect("fork ancestry is initialized");
                (height, hash)
            };
            if number >= height || height == 0 {
                return Err(Self::error(eyre!("block {number} is outside the fork ancestry")));
            }
            let block =
                block_on_handle(&self.handle, self.provider.get_block_by_hash(hash).into_future())
                    .map_err(Self::error)?
                    .ok_or_else(|| Self::error(eyre!("fork ancestor {hash} is unavailable")))?;
            if block.header.number() != height || block.header.hash != hash {
                return Err(Self::error(eyre!("fork ancestor does not match its requested hash")));
            }
            self.ancestry.lock().insert(height - 1, block.header.parent_hash());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Bytes, bytes};
    use alloy_rpc_types::{TransactionInput, TransactionRequest};
    use anvil::NodeConfig;

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_rpc_reads_pinned_storage_code_and_ancestry() {
        let (api, handle) = anvil::spawn(NodeConfig::test()).await;
        let address = Address::with_last_byte(0xc0);
        let code = bytes!("60003560005500");
        api.anvil_set_code(address, code.clone()).await.unwrap();
        api.anvil_set_storage_at(address, U256::ZERO, B256::from(U256::from(7))).await.unwrap();
        api.anvil_mine(Some(U256::from(3)), None).await.unwrap();
        let mut opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        opts.infer_network_from_fork().await.unwrap();
        let mut fork = EthereumFork::open(&Config::default(), &opts, None).await.unwrap();
        let provider = fork.fork.provider::<AnyNetwork>();
        let genesis = provider.get_block_by_number(0.into()).await.unwrap().unwrap();

        let transaction = TransactionRequest {
            from: Some(handle.dev_accounts().next().unwrap()),
            to: Some(address.into()),
            input: TransactionInput::new(Bytes::copy_from_slice(
                &U256::from(9).to_be_bytes::<32>(),
            )),
            gas: Some(100_000),
            gas_price: Some(2_000_000_000),
            ..Default::default()
        };
        provider.send_transaction(transaction.into()).await.unwrap().get_receipt().await.unwrap();

        assert_eq!(fork.database.get_storage(&address, &U256::ZERO).unwrap(), U256::from(7));
        let account = fork.database.get_account(&address).unwrap().unwrap();
        assert_eq!(account.code.unwrap().original_bytes(), code);
        assert_eq!(
            fork.database.get_code_by_hash(&account.code_hash).unwrap().original_bytes(),
            code
        );
        assert_eq!(fork.database.get_block_hash(&U256::ZERO).unwrap(), genesis.header.hash);
        assert_eq!(fork.env.block.number, U256::from(3));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_missing_code_is_fatal_instead_of_empty_bytecode() {
        let (_, handle) = anvil::spawn(NodeConfig::test()).await;
        let opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        let mut fork = EthereumFork::open(&Config::default(), &opts, None).await.unwrap();
        let error = fork.database.get_code_by_hash(&B256::with_last_byte(1)).unwrap_err();
        assert!(error.is_fatal());
        assert_eq!(
            error.to_string(),
            "RPC bytecode was not loaded for hash 0x0000000000000000000000000000000000000000000000000000000000000001"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fork_inferred_chain_id_follows_source_but_explicit_override_remains() {
        let (_, handle) = anvil::spawn(NodeConfig::test().with_chain_id(Some(31338u64))).await;
        let mut opts = EvmOpts { fork_url: Some(handle.http_endpoint()), ..Default::default() };
        opts.env.chain_id = Some(31337);
        opts.fork_chain_id_is_inferred = true;
        let fork = EthereumFork::open(&Config::default(), &opts, None).await.unwrap();
        assert_eq!(fork.env.version.chain_id, 31338);
        assert_eq!(fork.fork.context().source_chain_id, 31338);

        opts.fork_chain_id_is_inferred = false;
        let fork = EthereumFork::open(&Config::default(), &opts, None).await.unwrap();
        assert_eq!(fork.env.version.chain_id, 31337);
        assert_eq!(fork.fork.context().source_chain_id, 31338);
    }
}
