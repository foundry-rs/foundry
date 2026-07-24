use super::TxGenerator;
use crate::{CallDetails, FuzzFixtures, strategies::EvmFuzzState};
use alloy_json_abi::Function;
use alloy_primitives::Address;
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
    dictionary_weight: u32,
    payable_value_weight: u32,
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
            TxGenerator::call_strategy(
                &fuzz_state,
                &fuzz_fixtures,
                actual_target,
                func,
                dictionary_weight,
                payable_value_weight,
            )
        })
    })
}
