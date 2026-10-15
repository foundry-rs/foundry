//! Ethereum on evm2.

use super::{FoundryEvmFactory, FoundryEvmNetwork, FoundryTxEnv, build_eth_tx};
use crate::evm::EthEvmNetwork;
use alloy_consensus::transaction::Recovered;
use alloy_network::Ethereum;
use evm2::{
    BaseEvmTypes, Precompiles, SpecId, TxResult, Version, ethereum::TxEnvelope,
    evm::precompile::PrecompileProvider, registry::TxRegistry,
};

/// Ethereum evm2 factory over evm2's stock type family.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmFactory;

impl FoundryEvmNetwork for EthEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = EthEvmFactory;
}

impl FoundryEvmFactory for EthEvmFactory {
    type Types = BaseEvmTypes;
    type TxExt = ();

    fn version(&self, spec: SpecId, chain_id: u64) -> Version {
        let mut version = Version::new(spec);
        version.chain_id = chain_id;
        version
    }

    fn tx_registry(&self, spec: SpecId) -> TxRegistry<BaseEvmTypes, TxResult<BaseEvmTypes>> {
        evm2::ethereum::ethereum_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &super::EvmEnv<BaseEvmTypes>,
        _ext: &(),
    ) -> Box<dyn PrecompileProvider<BaseEvmTypes>> {
        Box::new(Precompiles::base(env.spec))
    }

    fn build_tx(&self, tx: &FoundryTxEnv) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(build_eth_tx(tx)?, tx.caller))
    }
}
