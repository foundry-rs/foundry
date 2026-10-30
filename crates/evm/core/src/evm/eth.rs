//! Ethereum on evm2.

use super::{
    EvmEnv, FoundryEvmFactory, FoundryEvmNetwork, TxPayload, eth_envelope_from_any_rpc, eth_tx,
    with_eip7702_delegations,
};
use alloy_consensus::transaction::Recovered;
use alloy_network::{AnyRpcTransaction, Ethereum};
use alloy_primitives::{Address, Bytes, U256, address};
use evm2::{
    BaseEvmTypes, Evm, EvmTypes, PrecompileError, Precompiles, SpecId, TxResult, Version,
    ethereum::TxEnvelope,
    evm::precompile::{PrecompileOutput, PrecompileProvider},
    interpreter::{GasTracker, Message},
    precompiles::{MovePrecompileError, PrecompileHalt, PrecompileId, PrecompileResult},
    registry::TxRegistry,
};
use foundry_evm_networks::{
    NetworkConfigs,
    celo::transfer::{CELO_TRANSFER_ADDRESS, CELO_TRANSFER_PRECOMPILE_NAME},
};

/// Gas Celo's transfer precompile charges.
const CELO_TRANSFER_GAS_COST: u64 = 9000;

/// Address of BSC's `P256VERIFY` precompile.
const BSC_P256_ADDRESS: Address = address!("0x0000000000000000000000000000000000000100");

const BSC_MAINNET_CHAIN_ID: u64 = 56;
const BSC_TESTNET_CHAIN_ID: u64 = 97;
const BSC_MAINNET_HABER_TIMESTAMP: u64 = 1_718_863_500;
const BSC_TESTNET_HABER_TIMESTAMP: u64 = 1_716_962_820;
const BSC_MAINNET_OSAKA_TIMESTAMP: u64 = 1_777_343_400;
const BSC_TESTNET_OSAKA_TIMESTAMP: u64 = 1_774_319_400;

/// Ethereum evm2 factory over evm2's stock type family.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmFactory {
    /// Whether Celo's transfer precompile is installed.
    celo: bool,
}

/// Ethereum EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmNetwork;

impl FoundryEvmNetwork for EthEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = EthEvmFactory;

    fn evm_tx(
        envelope: alloy_consensus::TxEnvelope,
        sender: Address,
        _payload: TxPayload,
    ) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(eth_tx(envelope), sender))
    }

    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<alloy_consensus::TxEnvelope> {
        eth_envelope_from_any_rpc(tx, "TxEnv")
    }
}

impl FoundryEvmFactory for EthEvmFactory {
    type Types = BaseEvmTypes;

    fn version(&self, spec: SpecId, chain_id: u64) -> Version {
        let mut version = Version::new(spec);
        version.chain_id = chain_id;
        with_eip7702_delegations(version)
    }

    fn tx_registry(&self, spec: SpecId) -> TxRegistry<BaseEvmTypes, TxResult<BaseEvmTypes>> {
        evm2::ethereum::ethereum_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<BaseEvmTypes>,
        _ext: &(),
    ) -> Box<dyn PrecompileProvider<BaseEvmTypes>> {
        let inner = Precompiles::base(env.spec);
        let p256 = bsc_p256(env.chain_id(), env.block.timestamp.saturating_to());
        if !self.celo && p256.is_none() {
            return Box::new(inner);
        }
        Box::new(EthPrecompiles { inner, celo: self.celo, p256 })
    }

    fn with_networks(&self, networks: NetworkConfigs) -> Self {
        Self { celo: networks.is_celo() }
    }
}

/// Ethereum precompiles with network additions: Celo's transfer precompile and BSC's
/// `P256VERIFY` rules.
struct EthPrecompiles {
    inner: Precompiles<BaseEvmTypes>,
    celo: bool,
    /// BSC's `P256VERIFY`, see [`bsc_p256`].
    p256: Option<Option<bool>>,
}

impl PrecompileProvider<BaseEvmTypes> for EthPrecompiles {
    fn addresses(&self) -> Vec<Address> {
        let mut addresses = self.inner.addresses();
        if self.celo {
            addresses.push(CELO_TRANSFER_ADDRESS);
        }
        if self.p256.is_some() {
            addresses.retain(|address| *address != BSC_P256_ADDRESS);
        }
        if let Some(Some(_)) = self.p256 {
            addresses.push(BSC_P256_ADDRESS);
        }
        addresses
    }

    fn precompile_ids(&self) -> Vec<(Address, PrecompileId)> {
        let mut ids = self.inner.precompile_ids();
        if self.celo {
            ids.push((CELO_TRANSFER_ADDRESS, PrecompileId::custom(CELO_TRANSFER_PRECOMPILE_NAME)));
        }
        if self.p256.is_some() {
            ids.retain(|(address, _)| *address != BSC_P256_ADDRESS);
        }
        if let Some(Some(_)) = self.p256 {
            ids.push((BSC_P256_ADDRESS, PrecompileId::P256Verify));
        }
        ids
    }

    fn contains(&self, address: &Address) -> bool {
        if self.celo && *address == CELO_TRANSFER_ADDRESS {
            return true;
        }
        if self.p256.is_some() && *address == BSC_P256_ADDRESS {
            return self.p256.flatten().is_some();
        }
        self.inner.contains(address)
    }

    fn move_precompiles(
        &mut self,
        moves: &[(Address, Address)],
    ) -> Result<(), MovePrecompileError> {
        self.inner.move_precompiles(moves)
    }

    fn execute(
        &mut self,
        evm: &mut Evm<'_, BaseEvmTypes>,
        message: &Message<BaseEvmTypes>,
        gas: &mut GasTracker,
    ) -> Option<PrecompileResult> {
        let target = message.call_target;
        if target == CELO_TRANSFER_ADDRESS {
            return self.celo.then(|| celo_transfer(evm, &message.input, gas));
        }
        if target == BSC_P256_ADDRESS
            && let Some(p256) = self.p256
        {
            return p256.map(|osaka| {
                if osaka {
                    evm2::precompiles::secp256r1::run_osaka(&message.input, gas)
                } else {
                    evm2::precompiles::secp256r1::run(&message.input, gas)
                }
            });
        }
        self.inner.execute(evm, message, gas)
    }
}

/// Runs Celo's native token transfer precompile.
///
/// See <https://specs.celo.org/token_duality.html#the-transfer-precompile>
pub fn celo_transfer<T: EvmTypes>(
    evm: &mut Evm<'_, T>,
    input: &[u8],
    gas: &mut GasTracker,
) -> PrecompileResult {
    let halt =
        |reason: &'static str| Err(PrecompileError::Halt(PrecompileHalt::Other(reason.into())));
    gas.spend(CELO_TRANSFER_GAS_COST)?;
    if input.len() != 96 {
        return halt("invalid input length for Celo transfer precompile");
    }
    let from = Address::from_slice(&input[12..32]);
    let to = Address::from_slice(&input[44..64]);
    let value = U256::from_be_slice(&input[64..96]);
    let state = evm.state_mut();
    let balance = state.account_info_untracked(&from)?.map(|info| info.balance);
    if balance.unwrap_or_default() < value {
        return halt("insufficient balance");
    }
    if !state.transfer(&from, &to, &value)? {
        return halt("balance overflow in to account");
    }
    Ok(PrecompileOutput::new(Bytes::new()))
}

/// Returns BSC's `P256VERIFY` on `chain_id` at `timestamp`. The outer option distinguishes BSC
/// chains from unrelated chains, while the inner option disables P256 before Haber and otherwise
/// tells whether it uses the Osaka gas schedule.
pub const fn bsc_p256(chain_id: u64, timestamp: u64) -> Option<Option<bool>> {
    let (haber_timestamp, osaka_timestamp) = match chain_id {
        BSC_MAINNET_CHAIN_ID => (BSC_MAINNET_HABER_TIMESTAMP, BSC_MAINNET_OSAKA_TIMESTAMP),
        BSC_TESTNET_CHAIN_ID => (BSC_TESTNET_HABER_TIMESTAMP, BSC_TESTNET_OSAKA_TIMESTAMP),
        _ => return None,
    };
    if timestamp < haber_timestamp { Some(None) } else { Some(Some(timestamp >= osaka_timestamp)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(factory: EthEvmFactory, spec: SpecId, chain_id: u64, timestamp: u64) -> Vec<Address> {
        let block = evm2::env::BlockEnv::<BaseEvmTypes> {
            timestamp: U256::from(timestamp),
            ..Default::default()
        };
        let env = EvmEnv::new(spec, factory.version(spec, chain_id), block);
        factory.precompiles(&env, &()).precompile_ids().into_iter().map(|(a, _)| a).collect()
    }

    #[test]
    fn bsc_precompile_overrides_stock_p256() {
        let eth = EthEvmFactory::default();
        // Osaka includes P256, but BSC removes it before Haber and adds it from Haber on.
        assert!(ids(eth, SpecId::OSAKA, 1, 0).contains(&BSC_P256_ADDRESS));
        assert!(
            !ids(eth, SpecId::OSAKA, 56, BSC_MAINNET_HABER_TIMESTAMP - 1)
                .contains(&BSC_P256_ADDRESS)
        );
        assert!(
            ids(eth, SpecId::CANCUN, 56, BSC_MAINNET_HABER_TIMESTAMP).contains(&BSC_P256_ADDRESS)
        );
        assert!(
            !ids(eth, SpecId::CANCUN, 1, BSC_MAINNET_HABER_TIMESTAMP).contains(&BSC_P256_ADDRESS)
        );
    }
}
