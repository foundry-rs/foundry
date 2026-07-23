use crate::{
    build::{LinkedBuildData, ScriptPredeployLibraries},
    execute::LinkedState,
    simulate::PreSimulationState,
};
use alloy_network::TransactionBuilder;
use alloy_primitives::{Address, keccak256};
use eyre::Result;
use foundry_common::{LIBRARY_DEPLOYER, matches_contract_creation, shell};
use foundry_compilers::ArtifactId;
use foundry_evm::{constants::CHEATCODE_ADDRESS, core::evm::FoundryEvmNetwork};
use std::collections::BTreeSet;

struct EligibleTransactions {
    required: BTreeSet<ArtifactId>,
    artifacts: Vec<ArtifactId>,
    rpc: String,
    baseline_count: usize,
}

impl<FEN: FoundryEvmNetwork> PreSimulationState<FEN> {
    /// Reruns narrowly eligible scripts with libraries unused by direct broadcasts deployed only
    /// in the local EVM.
    pub async fn optimize_library_deployments(self) -> Result<Self> {
        let Ok(Some(EligibleTransactions { required, artifacts, rpc, baseline_count })) =
            self.eligible()
        else {
            return Ok(self);
        };
        let onchain = match &self.build_data.predeploy_libraries {
            ScriptPredeployLibraries::Default { onchain, .. }
            | ScriptPredeployLibraries::Create2 { onchain, .. } => onchain,
        };
        if required.len() == onchain.len() {
            return Ok(self);
        }

        let Ok(libraries) = self.script_config.config.libraries_with_remappings() else {
            return Ok(self);
        };
        let linker = self.build_data.build_data.get_linker();
        let linked = match &self.build_data.predeploy_libraries {
            ScriptPredeployLibraries::Default { .. } => linker
                .link_with_partition(
                    libraries,
                    self.script_config.evm_opts.sender,
                    self.script_config.sender_nonce,
                    LIBRARY_DEPLOYER,
                    &required,
                    &self.build_data.build_data.target,
                )
                .map(|(output, local)| {
                    let onchain = onchain_libraries(&output, &local);
                    (output.output, ScriptPredeployLibraries::Default { onchain, local })
                }),
            ScriptPredeployLibraries::Create2 { salt, .. } => linker
                .link_with_create2_partition(
                    libraries,
                    self.script_config.evm_opts.create2_deployer,
                    *salt,
                    LIBRARY_DEPLOYER,
                    &required,
                    &self.build_data.build_data.target,
                )
                .map(|(output, local)| {
                    let onchain = onchain_libraries(&output, &local);
                    (
                        output.output,
                        ScriptPredeployLibraries::Create2 { onchain, salt: *salt, local },
                    )
                }),
        };
        let Ok((output, predeploy_libraries)) = linked else { return Ok(self) };
        let Ok(build_data) = LinkedBuildData::new(
            output.libraries,
            predeploy_libraries,
            self.build_data.build_data.clone(),
        ) else {
            return Ok(self);
        };
        let candidate = LinkedState {
            args: self.args.clone(),
            script_config: self.script_config.clone(),
            script_wallets: self.script_wallets.clone(),
            browser_wallet: self.browser_wallet.clone(),
            build_data,
        }
        .prepare_execution()
        .await;
        let Ok(candidate) = candidate else { return Ok(self) };
        let candidate = candidate.execute().await;
        let Ok(candidate) = candidate else { return Ok(self) };
        if !candidate.execution_result.success {
            return Ok(self);
        }
        let candidate = candidate.prepare_simulation_silent().await;
        let Ok(candidate) = candidate else { return Ok(self) };
        if self.execution_result.returned != candidate.execution_result.returned
            || self.execution_result.logs != candidate.execution_result.logs
            || !self.equivalent_candidate(&candidate, &artifacts, &rpc, baseline_count)
        {
            return Ok(self);
        }
        Ok(candidate)
    }

    fn eligible(&self) -> Result<Option<EligibleTransactions>> {
        if !self.execution_result.success
            || self.args.skip_simulation
            || self.args.debug
            || self.args.dump.is_some()
            || self.args.batch
            || self.args.slow
            || self.script_config.config.ffi
            || self.script_config.config.live_logs
            || shell::is_json()
            || shell::verbosity() > 3
            || self.script_config.evm_opts.env.gas_price.is_some()
            || self.execution_artifacts.rpc_data.missing_rpc
            || self.execution_artifacts.rpc_data.is_multi_chain()
            || self.script_config.evm_opts.sender == LIBRARY_DEPLOYER
            || self.script_config.config.fs_permissions.permissions.iter().any(|permission| {
                matches!(
                    permission.access,
                    foundry_config::fs_permissions::FsAccessPermission::Write
                        | foundry_config::fs_permissions::FsAccessPermission::ReadWrite
                )
            })
            || self.used_rerun_unsafe_cheatcode()
        {
            return Ok(None);
        }
        let Some(rpc) = self.script_config.evm_opts.fork_url.clone() else { return Ok(None) };
        if self.script_config.evm_opts.fork_block_number.is_none()
            || self.execution_artifacts.rpc_data.total_rpcs.len() != 1
            || !self.execution_artifacts.rpc_data.total_rpcs.contains(&rpc)
        {
            return Ok(None);
        }
        let onchain = match &self.build_data.predeploy_libraries {
            ScriptPredeployLibraries::Default { onchain, .. }
            | ScriptPredeployLibraries::Create2 { onchain, .. } => onchain,
        };
        if onchain.is_empty() {
            return Ok(None);
        }
        let Some(transactions) = &self.execution_result.transactions else { return Ok(None) };
        if transactions.len() <= onchain.len()
            || transactions.iter().any(|tx| {
                !tx.transaction.is_unsigned()
                    || tx.transaction.authorization_list().is_some_and(|list| !list.is_empty())
            })
        {
            return Ok(None);
        }
        for (index, (tx, library)) in transactions.iter().zip(onchain).enumerate() {
            let input = tx.transaction.input().map(|input| input.as_ref()).unwrap_or_default();
            let deployment_matches = match &self.build_data.predeploy_libraries {
                ScriptPredeployLibraries::Default { .. } => {
                    tx.transaction.to().is_none() && input == library.bytecode.as_ref()
                }
                ScriptPredeployLibraries::Create2 { salt, .. } => {
                    tx.transaction.to() == Some(self.script_config.evm_opts.create2_deployer)
                        && input.get(..32) == Some(salt.as_slice())
                        && input.get(32..) == Some(library.bytecode.as_ref())
                }
            };
            let exact = deployment_matches
                && tx.rpc.as_ref() == Some(&rpc)
                && tx.transaction.from() == Some(self.script_config.evm_opts.sender)
                && tx.transaction.nonce() == Some(self.script_config.sender_nonce + index as u64);
            if !exact {
                return Ok(None);
            }
        }

        let library_ids = onchain.iter().map(|library| library.id.clone()).collect::<BTreeSet<_>>();
        let linker = self.build_data.build_data.get_linker();
        let mut required = BTreeSet::new();
        let mut artifacts = Vec::new();
        for tx in transactions.iter().skip(onchain.len()) {
            if tx.rpc.as_ref() != Some(&rpc)
                || tx.transaction.from() != Some(self.script_config.evm_opts.sender)
                || tx.transaction.to().is_some()
            {
                return Ok(None);
            }
            let Some(input) = tx.transaction.input() else { return Ok(None) };
            let matches = self
                .build_data
                .known_contracts
                .iter()
                .filter(|(_, contract)| matches_contract_creation(contract, input))
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            let [artifact] = matches.as_slice() else { return Ok(None) };
            artifacts.push((*artifact).clone());
            required.extend(
                linker.dependencies(artifact)?.into_iter().filter(|id| library_ids.contains(id)),
            );
        }
        Ok(Some(EligibleTransactions { required, artifacts, rpc, baseline_count: onchain.len() }))
    }

    fn equivalent_candidate(
        &self,
        candidate: &Self,
        artifacts: &[ArtifactId],
        rpc: &str,
        baseline_count: usize,
    ) -> bool {
        let Some(baseline) = self.execution_result.transactions.as_ref() else { return false };
        let Some(candidate_txs) = candidate.execution_result.transactions.as_ref() else {
            return false;
        };
        let candidate_count = candidate.build_data.predeploy_libraries.libraries_count();
        let baseline = baseline.iter().skip(baseline_count).collect::<Vec<_>>();
        let candidate_txs = candidate_txs.iter().skip(candidate_count).collect::<Vec<_>>();
        if baseline.len() != artifacts.len() || candidate_txs.len() != artifacts.len() {
            return false;
        }
        let mut remaps = Vec::new();
        let baseline_libraries = match &self.build_data.predeploy_libraries {
            ScriptPredeployLibraries::Default { onchain, .. }
            | ScriptPredeployLibraries::Create2 { onchain, .. } => onchain,
        };
        let candidate_libraries = match &candidate.build_data.predeploy_libraries {
            ScriptPredeployLibraries::Default { onchain, .. }
            | ScriptPredeployLibraries::Create2 { onchain, .. } => onchain,
        };
        for library in baseline_libraries {
            if let Some(other) = candidate_libraries.iter().find(|other| other.id == library.id) {
                remaps.push((library.address, other.address));
            }
        }
        for ((baseline, candidate_tx), artifact) in
            baseline.iter().zip(&candidate_txs).zip(artifacts)
        {
            let (Some(mut old), Some(new)) = (
                baseline.transaction.clone().as_unsigned_mut().cloned(),
                candidate_tx.transaction.clone().as_unsigned_mut().cloned(),
            ) else {
                return false;
            };
            if baseline.rpc.as_deref() != Some(rpc)
                || candidate_tx.rpc.as_deref() != Some(rpc)
                || old.from() != new.from()
                || old.to().is_some()
                || new.to().is_some()
            {
                return false;
            }
            let (Some(old_nonce), Some(new_nonce)) = (old.nonce(), new.nonce()) else {
                return false;
            };
            if old_nonce.checked_sub(baseline_count as u64)
                != new_nonce.checked_sub(candidate_count as u64)
            {
                return false;
            }
            let mut input = old.input().cloned().unwrap_or_default().to_vec();
            for (from, to) in &remaps {
                replace_addresses(&mut input, *from, *to);
            }
            old.set_nonce(new_nonce);
            old.set_input(input);
            let (Ok(old_value), Ok(new_value)) =
                (serde_json::to_value(&old), serde_json::to_value(&new))
            else {
                return false;
            };
            if old_value != new_value
                || !candidate.build_data.known_contracts.get(artifact).is_some_and(|contract| {
                    new.input().is_some_and(|input| matches_contract_creation(contract, input))
                })
            {
                return false;
            }
            remaps.push((
                old.from().unwrap().create(old_nonce),
                new.from().unwrap().create(new_nonce),
            ));
        }
        true
    }

    fn used_rerun_unsafe_cheatcode(&self) -> bool {
        const SIGNATURES: &[&str] = &[
            "setEnv(string,string)",
            "rpc(string,string)",
            "rpc(string,string,string)",
            "rpcJson(string,string)",
            "rpcJson(string,string,string)",
            "sleep(uint256)",
            "prompt(string)",
            "promptSecret(string)",
            "promptSecretUint(string)",
            "promptAddress(string)",
            "promptUint(string)",
            "dumpState(string)",
            "createFork(string)",
            "createFork(string,uint256)",
            "createFork(string,bytes32)",
            "createSelectFork(string)",
            "createSelectFork(string,uint256)",
            "createSelectFork(string,bytes32)",
            "rollFork(uint256)",
            "rollFork(bytes32)",
            "rollFork(uint256,uint256)",
            "rollFork(uint256,bytes32)",
            "selectFork(uint256)",
            "transact(bytes32)",
            "transact(uint256,bytes32)",
        ];
        self.execution_result.traces.iter().any(|(_, traces)| {
            traces.nodes().iter().any(|node| {
                node.trace.address == CHEATCODE_ADDRESS
                    && node.trace.data.get(..4).is_some_and(|selector| {
                        SIGNATURES
                            .iter()
                            .any(|signature| &keccak256(signature.as_bytes())[..4] == selector)
                    })
            })
        })
    }
}

fn onchain_libraries(
    output: &foundry_linking::DetailedLinkOutput,
    local: &[foundry_linking::LinkedLibrary],
) -> Vec<foundry_linking::LinkedLibrary> {
    output
        .linked_libraries
        .iter()
        .filter(|library| !local.iter().any(|local| local.id == library.id))
        .cloned()
        .collect()
}

fn replace_addresses(input: &mut [u8], from: Address, to: Address) {
    let mut offset = 0;
    while let Some(index) =
        input[offset..].windows(Address::len_bytes()).position(|window| window == from.as_slice())
    {
        let start = offset + index;
        input[start..start + Address::len_bytes()].copy_from_slice(to.as_slice());
        offset = start + Address::len_bytes();
    }
}
