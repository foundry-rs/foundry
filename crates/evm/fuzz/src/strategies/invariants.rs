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
    // Each generated call owns its function-selection strategy. Share the functions so
    // constructing that strategy does not clone the entire target ABI on every call.
    let contracts = Arc::new(
        contracts
            .into_iter()
            .map(|(address, functions)| (address, Arc::new(functions)))
            .collect::<Vec<_>>(),
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_dyn_abi::JsonAbiExt;
    use proptest::{strategy::ValueTree, test_runner::TestRunner};

    #[test]
    fn override_uses_target_functions() {
        let contracts = vec![
            (
                Address::repeat_byte(1),
                vec![
                    Function::parse("first(uint256)").unwrap(),
                    Function::parse("second()").unwrap(),
                ],
            ),
            (Address::repeat_byte(2), vec![Function::parse("third(address)").unwrap()]),
        ];
        let target = Arc::new(RwLock::new(contracts[0].0));
        let strategy = override_call_strat(
            EvmFuzzState::test(),
            contracts.clone(),
            Arc::clone(&target),
            FuzzFixtures::default(),
            40,
            50,
        );
        let mut runner = TestRunner::deterministic();
        // Exercise changes to the preferred target and the fallback for an unknown target.
        for preferred in [contracts[0].0, contracts[1].0, Address::ZERO] {
            *target.write() = preferred;
            for _ in 0..64 {
                let mut tree = strategy.new_tree(&mut runner).unwrap();
                for _ in 0..16 {
                    let call = tree.current();
                    let (_, functions) = contracts.iter().find(|(a, _)| *a == call.target).unwrap();
                    let function =
                        functions.iter().find(|f| f.selector() == call.calldata[..4]).unwrap();
                    let values = function.abi_decode_input(&call.calldata[4..]).unwrap();
                    assert_eq!(function.abi_encode_input(&values).unwrap(), call.calldata);
                    assert_eq!(call.value, None);
                    if !tree.simplify() {
                        break;
                    }
                }
            }
        }
    }
}
