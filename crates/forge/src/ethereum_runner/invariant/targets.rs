//! Invariant targets resolved from setup state and the existing Solidity hooks.

use super::*;
use alloy_primitives::Selector;
use alloy_sol_types::SolCall;
use foundry_common::ContractsByAddress;
use foundry_evm_fuzz::invariant::{
    ArtifactFilters, IInvariantTest, SenderFilters, TargetedContracts,
};
use std::collections::btree_map::Entry;

pub(super) struct Targets {
    pub contracts: TargetedContracts,
    pub senders: SenderFilters,
    pub artifacts: ArtifactFilters,
    pub setup: ContractsByAddress,
    pub updatable: bool,
}

impl EthereumRunner {
    fn invariant_hook<C: SolCall>(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        call: C,
    ) -> Result<C::Return>
    where
        C::Return: Default,
    {
        let tx = self.tx(executor, CALLER, TxKind::Call(address), call.abi_encode().into())?;
        let (result, _) = executor.inspect_call(&tx, cheats.clone());
        let result = result?;
        Ok(if result.status {
            C::abi_decode_returns(&result.output).unwrap_or_default()
        } else {
            Default::default()
        })
    }

    pub(super) fn invariant_targets(
        &self,
        executor: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
    ) -> Result<Targets> {
        let mut artifacts = ArtifactFilters::default();
        for IInvariantTest::FuzzArtifactSelector { artifact, selectors } in self.invariant_hook(
            executor,
            cheats,
            address,
            IInvariantTest::targetArtifactSelectorsCall {},
        )? {
            let name = self.invariant_artifact(&artifact, &selectors)?;
            artifacts.targeted.entry(name).or_default().extend(selectors);
        }
        for artifact in
            self.invariant_hook(executor, cheats, address, IInvariantTest::excludeArtifactsCall {})?
        {
            artifacts.excluded.push(self.invariant_artifact(&artifact, &[])?);
        }
        for (id, contract) in self.artifacts.known_contracts.iter() {
            if !contract.abi.functions().any(|f| {
                !matches!(
                    f.state_mutability,
                    alloy_json_abi::StateMutability::View | alloy_json_abi::StateMutability::Pure
                )
            }) {
                artifacts.excluded.push(id.identifier());
            }
        }
        for artifact in
            self.invariant_hook(executor, cheats, address, IInvariantTest::targetArtifactsCall {})?
        {
            let name = self.invariant_artifact(&artifact, &[])?;
            if !artifacts.excluded.contains(&name) {
                artifacts.targeted.entry(name).or_default();
            }
        }
        let selected =
            self.invariant_hook(executor, cheats, address, IInvariantTest::targetContractsCall {})?;
        let excluded = self.invariant_hook(
            executor,
            cheats,
            address,
            IInvariantTest::excludeContractsCall {},
        )?;
        let mut setup = ContractsByAddress::default();
        let mut contracts = TargetedContracts::new();
        for account in executor.accepted_cache().accounts.keys() {
            let code = executor.code(*account)?;
            if let Some((id, data)) =
                self.artifacts.known_contracts.find_by_deployed_code(code.original_byte_slice())
            {
                setup.insert(*account, (id.identifier(), data.abi.clone()));
                if (*account == address && selected.contains(account))
                    || (*account != address
                        && *account != CHEATCODE_ADDRESS
                        && *account != foundry_evm::core::constants::HARDHAT_CONSOLE_ADDRESS
                        && (selected.is_empty() || selected.contains(account))
                        && !excluded.contains(account)
                        && artifacts.matches(&id.identifier()))
                {
                    let mut target = TargetedContract::new(id.identifier(), data.abi.clone())
                        .with_project_contracts(&self.artifacts.known_contracts);
                    target.targeted_functions =
                        artifacts.get_targeted_functions(id, &data.abi)?.unwrap_or_default();
                    target.rebuild_function_lookups();
                    contracts.inner.insert(*account, target);
                }
            }
        }
        for IInvariantTest::FuzzInterface { addr, artifacts } in
            self.invariant_hook(executor, cheats, address, IInvariantTest::targetInterfacesCall {})?
        {
            for artifact in artifacts {
                if let Some((_, data)) =
                    self.artifacts.known_contracts.find_by_name_or_identifier(&artifact)?
                {
                    let target = contracts.inner.entry(addr).or_insert_with(|| {
                        TargetedContract::new(artifact.clone(), data.abi.clone())
                    });
                    target.abi.functions.extend(data.abi.functions.clone());
                    target.rebuild_function_lookups();
                }
            }
        }
        for (account, (name, abi)) in &setup {
            if let Some(selectors) = artifacts.targeted.get(name)
                && !selectors.is_empty()
            {
                let target = contracts.inner.entry(*account).or_insert_with(|| {
                    TargetedContract::new(name.clone(), abi.clone())
                        .with_project_contracts(&self.artifacts.known_contracts)
                });
                target.add_selectors(selectors.iter().copied(), false)?;
            }
        }
        let targeted =
            self.invariant_hook(executor, cheats, address, IInvariantTest::targetSelectorsCall {})?;
        let explicit_test = targeted.iter().any(|f| f.addr == address && !f.selectors.is_empty());
        for (filters, exclude) in [
            (targeted, false),
            (
                self.invariant_hook(
                    executor,
                    cheats,
                    address,
                    IInvariantTest::excludeSelectorsCall {},
                )?,
                true,
            ),
        ] {
            for IInvariantTest::FuzzSelector { addr, selectors } in filters {
                if selectors.is_empty() {
                    continue;
                }
                let target = match contracts.inner.entry(addr) {
                    Entry::Occupied(entry) => entry.into_mut(),
                    Entry::Vacant(entry) => {
                        let (name, abi) = setup.get(&addr).ok_or_else(|| {
                            eyre::eyre!("address does not have an associated contract: {addr}")
                        })?;
                        entry.insert(TargetedContract::new(name.clone(), abi.clone()))
                    }
                };
                target.add_selectors(selectors, exclude)?;
            }
        }
        if !explicit_test && let Some(target) = contracts.inner.get_mut(&address) {
            let selectors = target
                .abi_fuzzed_functions()
                .filter(|f| !f.is_reserved())
                .map(|f| f.selector())
                .collect::<Vec<_>>();
            if selectors.is_empty() {
                contracts.inner.remove(&address);
            } else {
                target.add_selectors(selectors, false)?;
            }
        }
        ensure!(!contracts.is_empty(), "No contracts to fuzz.");
        ensure!(contracts.fuzzed_functions().next().is_some(), "No functions to fuzz.");
        let mut excluded_senders =
            self.invariant_hook(executor, cheats, address, IInvariantTest::excludeSendersCall {})?;
        excluded_senders.extend([
            CHEATCODE_ADDRESS,
            foundry_evm::core::constants::HARDHAT_CONSOLE_ADDRESS,
            DEFAULT_CREATE2_DEPLOYER,
        ]);
        excluded_senders.extend(foundry_evm::core::precompiles::PRECOMPILES);
        let senders = SenderFilters::new(
            self.invariant_hook(executor, cheats, address, IInvariantTest::targetSendersCall {})?,
            excluded_senders,
        );
        Ok(Targets { contracts, senders, artifacts, setup, updatable: selected.is_empty() })
    }

    fn invariant_artifact(&self, name: &str, selectors: &[Selector]) -> Result<String> {
        let (id, data) = self.artifacts.known_contracts.find_by_name_or_identifier(name)?.ok_or_else(|| eyre::eyre!("{name} not found in the project. Allowed format: `contract_name` or `contract_path:contract_name`."))?;
        for selector in selectors {
            ensure!(
                data.abi.functions().any(|f| f.selector() == *selector),
                "{name} does not have the selector {selector:?}"
            );
        }
        Ok(id.identifier())
    }
}
