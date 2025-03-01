use crate::{CallBuilder, Event, Interface, Result};
use alloy_dyn_abi::DynSolValue;
use alloy_json_abi::{Function, JsonAbi};
use alloy_network::{Ethereum, Network};
use alloy_primitives::{Address, Selector};
use alloy_provider::Provider;
use alloy_rpc_types_eth::Filter;
use alloy_sol_types::SolEvent;
use std::marker::PhantomData;

/// A handle to an Ethereum contract at a specific address.
///
/// A contract is an abstraction of an executable program on Ethereum. Every deployed contract has
/// an address, which is used to connect to it so that it may receive messages (transactions).
#[derive(Clone)]
pub struct ContractInstance<P, N = Ethereum> {
    address: Address,
    provider: P,
    interface: Interface,
    network: PhantomData<N>,
}

impl<P, N> ContractInstance<P, N> {
    /// Creates a new contract from the provided address, provider, and interface.
    #[inline]
    pub const fn new(address: Address, provider: P, interface: Interface) -> Self {
        Self { address, provider, interface, network: PhantomData }
    }

    /// Returns a reference to the contract's address.
    #[inline]
    pub const fn address(&self) -> &Address {
        &self.address
    }

    /// Sets the contract's address.
    #[inline]
    pub fn set_address(&mut self, address: Address) {
        self.address = address;
    }

    /// Returns a new contract instance at `address`.
    #[inline]
    pub fn at(mut self, address: Address) -> Self {
        self.set_address(address);
        self
    }

    /// Returns a reference to the contract's ABI.
    #[inline]
    pub const fn abi(&self) -> &JsonAbi {
        self.interface.abi()
    }

    /// Returns a reference to the contract's provider.
    #[inline]
    pub const fn provider(&self) -> &P {
        &self.provider
    }
}

impl<P: Clone, N> ContractInstance<&P, N> {
    /// Clones the provider and returns a new contract instance with the cloned provider.
    #[inline]
    pub fn with_cloned_provider(self) -> ContractInstance<P, N> {
        ContractInstance {
            address: self.address,
            provider: self.provider.clone(),
            interface: self.interface,
            network: PhantomData,
        }
    }
}

impl<P: Provider<N>, N: Network> ContractInstance<P, N> {
    /// Returns a transaction builder for the provided function name.
    ///
    /// If there are multiple functions with the same name due to overloading, consider using
    /// the [`ContractInstance::function_from_selector`] method instead, since this will use the
    /// first match.
    pub fn function(
        &self,
        name: &str,
        args: &[DynSolValue],
    ) -> Result<CallBuilder<(), &P, Function, N>> {
        let function = self.interface.get_from_name(name)?;
        CallBuilder::new_dyn(&self.provider, &self.address, function, args)
    }

    /// Returns a transaction builder for the provided function selector.
    pub fn function_from_selector(
        &self,
        selector: &Selector,
        args: &[DynSolValue],
    ) -> Result<CallBuilder<(), &P, Function, N>> {
        let function = self.interface.get_from_selector(selector)?;
        CallBuilder::new_dyn(&self.provider, &self.address, function, args)
    }

    /// Returns an [`Event`] builder with the provided filter.
    pub const fn event<E: SolEvent>(&self, filter: Filter) -> Event<(), &P, E, N> {
        Event::new(&self.provider, filter)
    }
}

impl<P, N> std::ops::Deref for ContractInstance<P, N> {
    type Target = Interface;

    fn deref(&self) -> &Self::Target {
        &self.interface
    }
}

impl<P, N> std::fmt::Debug for ContractInstance<P, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContractInstance").field("address", &self.address).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::TransactionBuilder;
    use alloy_primitives::{hex, U256};
    use alloy_provider::ProviderBuilder;
    use alloy_rpc_types_eth::TransactionRequest;

    #[tokio::test]
    async fn contract_interface() {
        let provider = ProviderBuilder::new().on_anvil_with_wallet();

        let abi_str = r#"[{"inputs":[],"name":"counter","outputs":[{"internalType":"uint256","name":"","type":"uint256"}],"stateMutability":"view","type":"function"},{"inputs":[],"name":"increment","outputs":[],"stateMutability":"nonpayable","type":"function"}]"#;
        let abi = serde_json::from_str::<JsonAbi>(abi_str).unwrap();
        let bytecode = hex::decode("6080806040523460135760b2908160188239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c90816361bc221a146065575063d09de08a14602f575f80fd5b346061575f3660031901126061575f5460018101809111604d575f55005b634e487b7160e01b5f52601160045260245ffd5b5f80fd5b346061575f3660031901126061576020905f548152f3fea2646970667358221220d802267a5f574e54a87a63d0ff8d733fdb275e6e6c502831d9e14f957bbcd7a264736f6c634300081a0033").unwrap();
        let deploy_tx = TransactionRequest::default().with_deploy_code(bytecode);
        let address = provider
            .send_transaction(deploy_tx)
            .await
            .unwrap()
            .get_receipt()
            .await
            .unwrap()
            .contract_address
            .unwrap();

        let contract = ContractInstance::new(address, provider, Interface::new(abi));
        assert_eq!(contract.abi().functions().count(), 2);

        let result = contract.function("counter", &[]).unwrap().call().await.unwrap();
        assert_eq!(result[0].as_uint().unwrap().0, U256::from(0));

        contract.function("increment", &[]).unwrap().send().await.unwrap().watch().await.unwrap();

        let result = contract.function("counter", &[]).unwrap().call().await.unwrap();
        assert_eq!(result[0].as_uint().unwrap().0, U256::from(1));
    }
}
