//! Monad on evm2, using `monad-evm2`.

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork, FoundryTxEnv,
    FoundryTxResultExt, build_eth_tx,
};
use crate::evm::MonadEvmNetwork;
use alloy_consensus::transaction::Recovered;
use alloy_network::Ethereum;
use evm2::{
    TxResult, Version, ethereum::TxEnvelope, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
use monad_evm2::{
    MonadBlockExt, MonadChainContext, MonadEvmExt, MonadEvmFactory, MonadEvmTypes, MonadHardfork,
    MonadPrecompiles, MonadTxResultExt, monad_tx_registry, monad_version,
};

impl FoundryBlockExt for MonadBlockExt {}

impl FoundryEvmExt<()> for MonadEvmExt {
    fn for_transaction(&mut self, tx: &FoundryTxEnv) {
        self.chain = MonadChainContext {
            current_block_senders: vec![tx.caller],
            current_block_authorities: vec![
                tx.authorization_list.iter().filter_map(|auth| auth.authority()).collect(),
            ],
            current_tx_index: 0,
            ..Default::default()
        };
    }
}

impl FoundryTxResultExt for MonadTxResultExt {}

impl FoundryEvmNetwork for MonadEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = MonadEvmFactory;
}

impl FoundryEvmFactory for MonadEvmFactory {
    type Types = MonadEvmTypes;
    type TxExt = ();

    fn version(&self, spec: MonadHardfork, chain_id: u64) -> Version {
        monad_version(spec, chain_id)
    }

    fn tx_registry(
        &self,
        spec: MonadHardfork,
    ) -> TxRegistry<MonadEvmTypes, TxResult<MonadEvmTypes>> {
        monad_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<MonadEvmTypes>,
        _ext: &MonadEvmExt,
    ) -> Box<dyn PrecompileProvider<MonadEvmTypes>> {
        Box::new(MonadPrecompiles::new(env.spec))
    }

    fn build_tx(&self, tx: &FoundryTxEnv) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(build_eth_tx(tx)?, tx.caller))
    }
}
