use ethers::{
    abi::{Abi, Address, Detokenize, FixedBytes, Function, Tokenizable, TokenizableItem},
    prelude::U256,
};
use eyre::ContextCompat;
use revm::db::DatabaseRef;
use tracing::warn;

use super::{InvariantExecutor, TargetedContracts};

impl<'a, DB> InvariantExecutor<'a, DB>
where
    DB: DatabaseRef + Clone,
{
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
                    *addr !=
                        Address::from_slice(
                            &hex::decode("7109709ECfa91a80626fF3989D68f67F5b1DD12D").unwrap(),
                        ) &&
                    *addr !=
                        Address::from_slice(
                            &hex::decode("000000000000000000636F6e736F6c652e6c6f67").unwrap(),
                        ) &&
                    (selected.is_empty() || selected.contains(addr)) &&
                    (excluded.is_empty() || !excluded.contains(addr))
            })
            .map(|(addr, (name, abi))| (addr, (name, abi, vec![])))
            .collect();

        self.select_selectors(invariant_address, abi, &mut contracts)?;

        Ok((senders, contracts))
    }

    /// Selects the functions to fuzz based on the contract method `targetSelectors() -> (address,
    /// bytes4)[]`.
    pub fn select_selectors(
        &self,
        address: Address,
        abi: &Abi,
        targeted_contracts: &mut TargetedContracts,
    ) -> eyre::Result<()> {
        let selectors =
            self.get_list::<(Address, Vec<FixedBytes>)>(address, abi, "targetSelectors");

        fn add_function(
            name: &str,
            selector: FixedBytes,
            abi: &Abi,
            funcs: &mut Vec<Function>,
        ) -> eyre::Result<()> {
            let func = abi
                .functions()
                .into_iter()
                .find(|func| func.short_signature().as_slice() == selector.as_slice())
                .wrap_err(format!("{name} does not have the selector {:?}", selector))?;

            funcs.push(func.clone());
            Ok(())
        }

        for (address, bytes4_array) in selectors.into_iter() {
            if let Some((name, abi, address_selectors)) = targeted_contracts.get_mut(&address) {
                for selector in bytes4_array {
                    add_function(name, selector, abi, address_selectors)?;
                }
            } else {
                let (name, abi) = self.setup_contracts.get(&address).wrap_err(format!(
                    "[targetSelectors] address does not have an associated contract: {}",
                    address
                ))?;
                let mut functions = vec![];
                for selector in bytes4_array {
                    add_function(name, selector, abi, &mut functions)?;
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
        let mut list: Vec<T> = vec![];

        if let Some(func) = abi.functions().into_iter().find(|func| func.name == method_name) {
            if let Ok(call_result) = self.evm.call::<Vec<T>, _, _>(
                self.sender,
                address,
                func.clone(),
                (),
                U256::zero(),
                Some(abi),
            ) {
                list = call_result.result;
            } else {
                warn!(
                    "The function {} was found but there was an error querying its data.",
                    method_name
                );
            }
        };

        list
    }
}
