use super::{TxGenerator, WeightedIndices};
use crate::{
    CallDetails, FuzzFixtures,
    strategies::{DictionaryRead, EvmFuzzState},
};
use alloy_json_abi::Function;
use alloy_primitives::Address;
use parking_lot::RwLock;
use proptest::prelude::*;
use rand::seq::IteratorRandom;
use std::sync::Arc;

/// Given a target address, we generate random calldata.
pub fn override_call_strat(
    fuzz_state: EvmFuzzState,
    contracts: Vec<(Address, Vec<(String, Function)>)>,
    target: Arc<RwLock<Address>>,
    fuzz_fixtures: FuzzFixtures,
    dictionary_weight: u32,
    payable_value_weight: u32,
) -> impl Strategy<Value = CallDetails> + Send + Sync + 'static {
    // Each generated call owns its function-selection strategy. Share the functions so
    // constructing that strategy does not clone the entire target ABI on every call.
    let guidance = fuzz_state.with_dictionary(|dict| Arc::clone(dict.guidance()));
    let contracts = Arc::new(
        contracts
            .into_iter()
            .map(|(address, identified_functions)| {
                let weighted_indices = guidance.has_selector_weights().then(|| {
                    WeightedIndices::from_weights(identified_functions.iter().map(
                        |(identifier, function)| {
                            guidance.selector_weight(identifier, function).unwrap_or(1)
                        },
                    ))
                });
                let functions = identified_functions
                    .into_iter()
                    .map(|(_, function)| function)
                    .collect::<Vec<_>>();
                (address, Arc::new(functions), weighted_indices)
            })
            .collect::<Vec<_>>(),
    );
    let contracts_ref = contracts.clone();
    proptest::prop_oneof![
        80 => proptest::strategy::LazyJust::new(move || *target.read()),
        20 => any::<prop::sample::Selector>()
            .prop_map(move |selector| {
                let (target, _, _) = selector.select(contracts_ref.iter());
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
            let (actual_target, fuzzed_functions, weighted_indices) = contracts
                .iter()
                .find(|(address, _, _)| *address == target_address)
                .map(|(address, functions, weights)| (*address, functions.clone(), weights.clone()))
                .unwrap_or_else(|| {
                    let (address, functions, weights) = contracts
                        .iter()
                        .choose(&mut rand::rng())
                        .expect("at least one target contract");
                    (*address, functions.clone(), weights.clone())
                });
            (
                actual_target,
                any::<prop::sample::Index>().prop_map(move |index| {
                    let selected = weighted_indices.as_ref().map_or_else(
                        || index.index(fuzzed_functions.len()),
                        |weights| weights.select(index, fuzzed_functions.len()),
                    );
                    fuzzed_functions[selected].clone()
                }),
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
