use super::{TxGenerator, fuzz_contract_with_calldata};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
    strategies::{EvmFuzzState, InvariantFuzzState},
};
use alloy_json_abi::Function;
use alloy_primitives::Address;
use foundry_config::InvariantConfig;
use parking_lot::RwLock;
use proptest::prelude::*;
use rand::seq::IteratorRandom;
use std::sync::Arc;

/// Given a target address, we generate random calldata.
pub fn override_call_strat(
    fuzz_state: EvmFuzzState,
    contracts: Vec<(Address, Vec<Function>)>,
    target: Arc<RwLock<Address>>,
    fuzz_fixtures: FuzzFixtures,
) -> impl Strategy<Value = CallDetails> + Send + Sync + 'static {
    let contracts = Arc::new(contracts);
    let contracts_ref = contracts.clone();
    proptest::prop_oneof![
        80 => proptest::strategy::LazyJust::new(move || *target.read()),
        20 => any::<prop::sample::Selector>()
            .prop_map(move |selector| {
                let (target, _) = selector.select(contracts_ref.iter());
                *target
            }),
    ]
    .prop_flat_map(move |target_address| {
        let fuzz_state = fuzz_state.clone();
        let fuzz_fixtures = fuzz_fixtures.clone();
        let contracts = contracts.clone();

        let (actual_target, func) = {
            // If the target address is in the contracts map, use it directly.
            // Otherwise, fall back to a random contract from the targeted contracts.
            // This can happen when call_override sets target_reference to a contract
            // that is not in targetContracts (e.g., the protocol contract during reentrancy).
            let (actual_target, fuzzed_functions) = contracts
                .iter()
                .find(|(address, _)| *address == target_address)
                .map(|(address, functions)| (*address, functions.clone()))
                .unwrap_or_else(|| {
                    let (address, functions) = contracts
                        .iter()
                        .choose(&mut rand::rng())
                        .expect("at least one target contract");
                    (*address, functions.clone())
                });
            (
                actual_target,
                any::<prop::sample::Index>()
                    .prop_map(move |index| index.get(&fuzzed_functions).clone()),
            )
        };

        func.prop_flat_map(move |func| {
            fuzz_contract_with_calldata(&fuzz_state, &fuzz_fixtures, actual_target, func)
        })
    })
}

/// Creates the invariant strategy.
///
/// Given the known and future contracts, it generates the next call by fuzzing the `caller`,
/// `calldata` and `target`. The generated data is evaluated lazily for every single call to fully
/// leverage the evolving fuzz dictionary.
///
/// The fuzzed parameters can be filtered through different methods implemented in the test
/// contract:
///
/// `targetContracts()`, `targetSenders()`, `excludeContracts()`, `targetSelectors()`
pub fn invariant_strat(
    fuzz_state: InvariantFuzzState,
    senders: SenderFilters,
    contracts: FuzzRunIdentifiedContracts,
    config: InvariantConfig,
    fuzz_fixtures: FuzzFixtures,
) -> impl Strategy<Value = BasicTxDetails> {
    TxGenerator::invariant(fuzz_state, senders, contracts, config, fuzz_fixtures).strategy()
}
