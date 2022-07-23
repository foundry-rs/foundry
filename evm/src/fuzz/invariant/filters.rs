use ethers::{
    abi::{Abi, Address, Detokenize, FixedBytes, Function, Tokenizable, TokenizableItem},
    prelude::U256,
};
use eyre::ContextCompat;

use tracing::warn;

use crate::{
    executor::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    CALLER,
};

use super::{InvariantExecutor, TargetedContracts};

impl<'a> InvariantExecutor<'a> {
    /// Selects senders and contracts based on the contract methods `targetSenders() -> address[]`,
    /// `targetContracts() -> address[]` and `excludeContracts() -> address[]`.
    pub fn select_contracts_and_senders(
        &self,
        invariant_address: Address,
        abi: &Abi,
    ) -> eyre::Result<(Vec<Address>, TargetedContracts)> {
        let [senders, selected, excluded] =
            ["targetSenders", "targetContracts", "excludeContracts"]
                .map(|method| self.get_list::<Address>(invariant_address, abi, method));

        let mut contracts: TargetedContracts = self
            .setup_contracts
            .clone()
            .into_iter()
            .filter(|(addr, _)| {
                *addr != invariant_address &&
                    *addr != CHEATCODE_ADDRESS &&
                    *addr != HARDHAT_CONSOLE_ADDRESS &&
                    (selected.is_empty() || selected.contains(addr)) &&
                    (excluded.is_empty() || !excluded.contains(addr))
            })
            .map(|(addr, (name, abi))| (addr, (name, abi, vec![])))
            .collect();

        self.select_selectors(invariant_address, abi, &mut contracts)?;

        Ok((senders, contracts))
    }

    /// Selects the functions to fuzz based on the contract method `targetSelectors() -> (address,
    /// bytes4[])[]`.
    pub fn select_selectors(
        &self,
        address: Address,
        abi: &Abi,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        let selectors =
            self.get_list::<(Address, Vec<FixedBytes>)>(address, abi, "targetSelectors");

        fn get_function(name: &str, selector: FixedBytes, abi: &Abi) -> eyre::Result<Function> {
            abi.functions()
                .into_iter()
                .find(|func| func.short_signature().as_slice() == selector.as_slice())
                .cloned()
                .wrap_err(format!("{name} does not have the selector {:?}", selector))
        }

        for (address, bytes4_array) in selectors.into_iter() {
            if let Some((name, abi, address_selectors)) = targeted_contracts.get_mut(&address) {
                // The contract is already part of our filter, and all we do is specify that we're
                // only looking at specific functions coming from `bytes4_array`.
                for selector in bytes4_array {
                    address_selectors.push(get_function(name, selector, abi)?);
                }
            } else {
                let (name, abi) = self.setup_contracts.get(&address).wrap_err(format!(
                    "[targetSelectors] address does not have an associated contract: {}",
                    address
                ))?;
                let mut functions = vec![];
                for selector in bytes4_array {
                    functions.push(get_function(name, selector, abi)?);
                }

                targeted_contracts.insert(address, (name.to_string(), abi.clone(), functions));
            }
        }
        Ok(())
    }

    /// Gets list of `T` by calling the contract `method_name` function.
    fn get_list<T>(&self, address: Address, abi: &Abi, method_name: &str) -> Vec<T>
    where
        T: Tokenizable + Detokenize + TokenizableItem,
    {
        if let Some(func) = abi.functions().into_iter().find(|func| func.name == method_name) {
            if let Ok(call_result) = self.executor.call::<Vec<T>, _, _>(
                CALLER,
                address,
                func.clone(),
                (),
                U256::zero(),
                Some(abi),
            ) {
                return call_result.result
            } else {
                warn!(
                    "The function {} was found but there was an error querying its data.",
                    method_name
                );
            }
        };

        Vec::new()
    }
}
