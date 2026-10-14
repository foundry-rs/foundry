//! Failure replay and sequence reduction over independently owned setup state.

use super::*;
use alloy_primitives::Selector;
use foundry_evm_fuzz::CallDetails;
use std::path::Path;

pub(super) struct Failure {
    pub site: FailureSite,
    pub result: TestResult,
    pub calls: Vec<BasicTxDetails>,
    pub call_overrides: Vec<Vec<Option<BasicTxDetails>>>,
    pub assertion: bool,
}

impl EthereumRunner {
    #[expect(
        clippy::too_many_arguments,
        reason = "Replay must use the exact setup, targeting and predicate policy."
    )]
    pub(super) fn replay_invariant(
        &self,
        base: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        function: &Function,
        config: &Config,
        abi: &JsonAbi,
        targets: &targets::Targets,
        sequence: &[BasicTxDetails],
        call_overrides: &[Vec<Option<BasicTxDetails>>],
    ) -> Result<Option<Failure>> {
        if (!call_overrides.is_empty() && call_overrides.len() != sequence.len())
            || (!config.invariant.call_override
                && call_overrides.iter().any(|calls| !calls.is_empty()))
        {
            return Ok(None);
        }
        let mut executor = base.clone();
        let mut inspector = cheats.clone();
        let contracts =
            FuzzRunIdentifiedContracts::new(targets.contracts.clone(), targets.updatable);
        let initial = self.check_invariant(base, cheats, address, function, config, abi)?;
        if initial.status.is_failure() {
            return Ok(Some(Failure {
                site: FailureSite::Predicate(function.selector()),
                result: initial,
                calls: Vec::new(),
                call_overrides: Vec::new(),
                assertion: false,
            }));
        }
        let mut replayed = Vec::new();
        for (index, input) in sequence.iter().enumerate() {
            if let Some(generator) =
                inspector.fuzzer.as_mut().and_then(|fuzzer| fuzzer.call_generator.as_mut())
            {
                let decisions = call_overrides.get(index).cloned().unwrap_or_default();
                generator.last_sequence = Arc::new(RwLock::new(decisions));
                generator.replay_exhausted = false;
                generator.set_replay(true);
                *generator.handler_addresses.write() =
                    contracts.targets().keys().copied().collect();
            }
            if !targets.senders.allows(input.sender) || !contracts.targets().can_replay(input) {
                return Ok(None);
            }
            let mut input = input.clone();
            let (result, observed) =
                self.invariant_call(&mut executor, &inspector, &mut input, None)?;
            if observed
                .fuzzer
                .as_ref()
                .and_then(|fuzzer| fuzzer.call_generator.as_ref())
                .is_some_and(|generator| {
                    generator.replay_exhausted || !generator.last_sequence.read().is_empty()
                })
            {
                return Ok(None);
            }
            if result.output.as_ref() == foundry_evm::core::constants::MAGIC_ASSUME {
                return Ok(None);
            }
            let failed_assertion =
                self.invariant_handler_assertion(&executor, &result, &observed)?;
            replayed.push(input);
            if failed_assertion || (!result.status && config.invariant.fail_on_revert) {
                let call = replayed.last().expect("replayed call");
                let site = FailureSite::Handler(
                    call.call_details.target,
                    observed.reverter.unwrap_or(call.call_details.target),
                    Selector::from_slice(&call.call_details.calldata[..4]),
                );
                let result =
                    self.outcome(result, failed_assertion, 0, observed, Default::default());
                return Ok(Some(Failure {
                    site,
                    result,
                    call_overrides: call_overrides
                        .get(..replayed.len())
                        .unwrap_or_default()
                        .to_vec(),
                    calls: replayed,
                    assertion: failed_assertion,
                }));
            }
            inspector = observed;
            self.discover_invariant_contracts(&executor, targets, &contracts)?;
            let interval = config.invariant.check_interval;
            if (interval > 0 && replayed.len().is_multiple_of(interval as usize))
                || replayed.len() == sequence.len()
            {
                let result =
                    self.check_invariant(&executor, &inspector, address, function, config, abi)?;
                if result.status.is_failure() {
                    return Ok(Some(Failure {
                        site: FailureSite::Predicate(function.selector()),
                        result,
                        call_overrides: call_overrides
                            .get(..replayed.len())
                            .unwrap_or_default()
                            .to_vec(),
                        calls: replayed,
                        assertion: false,
                    }));
                }
            }
        }
        if let Some(hook) = abi.functions.get("afterInvariant").and_then(|hooks| hooks.first()) {
            let result = self.check_invariant(&executor, &inspector, address, hook, config, abi)?;
            if result.status.is_failure() {
                return Ok(Some(Failure {
                    site: FailureSite::AfterInvariant,
                    result,
                    call_overrides: call_overrides
                        .get(..replayed.len())
                        .unwrap_or_default()
                        .to_vec(),
                    calls: replayed,
                    assertion: false,
                }));
            }
        }
        Ok(None)
    }

    pub(super) fn invariant_handler_assertion(
        &self,
        executor: &Executor<Database>,
        result: &TxResult<FoundryEvmTypes>,
        cheats: &Cheatcodes,
    ) -> Result<bool> {
        let panic_assertion = result.output.len() == 36
            && result.output[..4] == [0x4e, 0x48, 0x7b, 0x71]
            && U256::from_be_slice(&result.output[4..]) == U256::ONE;
        Ok(executor.storage(CHEATCODE_ADDRESS, GLOBAL_FAIL_SLOT)? != U256::ZERO
            || cheats.snapshot_failure
            || (!result.status
                && (panic_assertion
                    || self
                        .artifacts
                        .revert_decoder
                        .decode_data(&result.output)
                        .starts_with(foundry_evm::core::decode::ASSERTION_FAILED_PREFIX))))
    }

    pub(super) fn discover_invariant_contracts(
        &self,
        executor: &Executor<Database>,
        targets: &targets::Targets,
        contracts: &FuzzRunIdentifiedContracts,
    ) -> Result<()> {
        if !targets.updatable {
            return Ok(());
        }
        for account in executor
            .accepted_cache()
            .accounts
            .keys()
            .filter(|account| !targets.setup.contains_key(*account))
        {
            let code = executor.code(*account)?;
            if let Some((id, data)) =
                self.artifacts.known_contracts.find_by_deployed_code(code.original_byte_slice())
                && let Some(functions) = targets.artifacts.get_targeted_functions(id, &data.abi)?
            {
                let mut contract = TargetedContract::new(id.name.clone(), data.abi.clone())
                    .with_project_contracts(&self.artifacts.known_contracts);
                contract.targeted_functions = functions;
                contract.rebuild_function_lookups();
                contracts.add_created_contract(*account, contract);
            }
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Every candidate must execute with the original setup and failure policy."
    )]
    pub(super) fn minimize_invariant(
        &self,
        base: &Executor<Database>,
        cheats: &Cheatcodes,
        address: Address,
        function: &Function,
        config: &Config,
        abi: &JsonAbi,
        targets: &targets::Targets,
        mut failure: Failure,
    ) -> Result<Failure> {
        let original_site = failure.site;
        let original_assertion = failure.assertion;
        let original_reason = failure.result.reason.clone();
        let mut attempts = 0u32;
        let mut chunk = failure.calls.len();
        while chunk > 0 && attempts < config.invariant.shrink_run_limit {
            let mut start = 0;
            let mut shortened = false;
            while start < failure.calls.len() && attempts < config.invariant.shrink_run_limit {
                let end = (start + chunk).min(failure.calls.len());
                let mut candidate = failure.calls.clone();
                let removed = candidate.drain(start..end).collect::<Vec<_>>();
                let mut candidate_overrides = failure.call_overrides.clone();
                if !candidate_overrides.is_empty() {
                    candidate_overrides.drain(start..end);
                }
                if config.invariant.has_delay()
                    && let Some(next) = candidate.get_mut(start)
                {
                    for call in removed {
                        if let Some(warp) = call.warp {
                            next.warp = Some(next.warp.unwrap_or_default().saturating_add(warp));
                        }
                        if let Some(roll) = call.roll {
                            next.roll = Some(next.roll.unwrap_or_default().saturating_add(roll));
                        }
                    }
                }
                attempts += 1;
                if let Some(replayed) = self.replay_invariant(
                    base,
                    cheats,
                    address,
                    function,
                    config,
                    abi,
                    targets,
                    &candidate,
                    &candidate_overrides,
                )? && replayed.site == original_site
                    && replayed.result.reason == original_reason
                    && replayed.assertion == original_assertion
                {
                    failure = replayed;
                    shortened = true;
                } else {
                    start += chunk;
                }
            }
            if !shortened {
                chunk /= 2;
            }
        }
        Ok(failure)
    }
}

pub(super) fn persisted_failure(
    path: &Path,
    settings: &InvariantSettings,
) -> Option<InvariantPersistedFailure> {
    let failure = foundry_common::fs::read_json_file::<InvariantPersistedFailure>(path).ok()?;
    // Symbolic assignments and edge provenance require their original replay owner.
    if !failure.storage.is_empty()
        || failure.failure_site.is_some()
        || failure.fingerprint_provenance.is_some()
    {
        return None;
    }
    if let Some(diff) = failure.settings.diff(settings) {
        let _ = sh_warn!(
            "Failure from {path:?} file was ignored because invariant test settings have changed: {diff}"
        );
        return None;
    }
    Some(failure)
}

pub(super) fn failure_calls(failure: &InvariantPersistedFailure) -> Option<Vec<BasicTxDetails>> {
    failure
        .call_sequence
        .iter()
        .map(|call| {
            Some(BasicTxDetails {
                warp: call.warp,
                roll: call.roll,
                sender: call.sender?,
                call_details: CallDetails {
                    target: call.addr?,
                    calldata: call.calldata.clone(),
                    value: call.value,
                },
            })
        })
        .collect()
}
