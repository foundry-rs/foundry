use super::{fuzz_calldata, fuzz_msg_value, fuzz_param_from_state};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
    strategies::{
        EvmFuzzState, FuzzStateReader, InvariantFuzzState, fuzz_calldata_from_state, fuzz_param,
    },
};
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256, address};
use foundry_config::InvariantConfig;
use foundry_evm_core::constants::CALLER;
use parking_lot::RwLock;
use proptest::prelude::*;
use rand::seq::IteratorRandom;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[derive(Default)]
struct PlannedFuzzedCalls {
    generation: u64,
    calls: Vec<BoxedStrategy<CallDetails>>,
}

/// Default invariant senders, modeled after Echidna's fixed sender pool.
///
/// The Foundry default deployer is included because owner/deployer-only paths are common in
/// invariant targets.
const DEFAULT_INVARIANT_SENDERS: [Address; 3] = [
    address!("0x0000000000000000000000000000000000010000"),
    address!("0x0000000000000000000000000000000000020000"),
    CALLER,
];

const RANDOM_SENDER_WEIGHT: u32 = 1;
const DEFAULT_SENDER_WEIGHT: u32 = 99;

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
            fuzz_contract_with_calldata(
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
    let senders = Rc::new(senders);
    let dictionary_weight = config.dictionary.dictionary_weight;
    let payable_value_weight = config.corpus.payable_value_weight;
    let planned_calls = Rc::new(RefCell::new(PlannedFuzzedCalls::default()));

    // Strategy to generate values for tx warp and roll.
    let warp_roll_strat = |cond: bool| {
        if cond { any::<U256>().prop_map(Some).boxed() } else { Just(None).boxed() }
    };

    any::<prop::sample::Selector>()
        .prop_flat_map(move |selector| {
            let sender = select_random_sender(&fuzz_state, senders.clone(), dictionary_weight);
            let call_details = {
                let generation = contracts.fuzzed_functions_generation();
                let mut planned_calls = planned_calls.borrow_mut();
                if planned_calls.generation != generation || planned_calls.calls.is_empty() {
                    let functions = contracts.fuzzed_functions();
                    planned_calls.calls = functions
                        .iter()
                        .map(|(target_address, target_function)| {
                            fuzz_contract_with_calldata(
                                &fuzz_state,
                                &fuzz_fixtures,
                                *target_address,
                                target_function.clone(),
                                dictionary_weight,
                                payable_value_weight,
                            )
                            .boxed()
                        })
                        .collect();
                    planned_calls.generation = generation;
                }
                selector.select(planned_calls.calls.iter()).clone()
            };

            let warp = warp_roll_strat(config.max_time_delay.is_some());
            let roll = warp_roll_strat(config.max_block_delay.is_some());

            (warp, roll, sender, call_details)
        })
        .prop_map(move |(warp, roll, sender, call_details)| {
            let warp =
                warp.map(|time| time % U256::from(config.max_time_delay.unwrap_or_default()));
            let roll =
                roll.map(|block| block % U256::from(config.max_block_delay.unwrap_or_default()));
            BasicTxDetails { warp, roll, sender, call_details }
        })
}

/// Strategy to select a sender address:
/// * If `senders` is empty, then it is usually sampled from Foundry's fixed default sender pool. A
///   random or dictionary address is used rarely to preserve broad exploration.
/// * If `senders` is not empty, a random address is chosen from the list of senders.
fn select_random_sender<S: FuzzStateReader>(
    fuzz_state: &S,
    senders: Rc<SenderFilters>,
    dictionary_weight: u32,
) -> impl Strategy<Value = Address> + use<S> {
    if senders.targeted.is_empty() {
        let default_senders = default_invariant_senders(&senders);
        let dictionary_weight = dictionary_weight.min(100);
        let random_sender = proptest::prop_oneof![
            100 - dictionary_weight => fuzz_param(&alloy_dyn_abi::DynSolType::Address),
            dictionary_weight => fuzz_param_from_state(&alloy_dyn_abi::DynSolType::Address, fuzz_state),
        ]
        .prop_map(move |addr| {
            let mut addr = addr.as_address().unwrap();
            // Make sure the selected address is not in the list of excluded senders.
            // We don't use proptest's filter to avoid reaching the `PROPTEST_MAX_LOCAL_REJECTS`
            // max rejects and exiting test before all runs completes.
            // See <https://github.com/foundry-rs/foundry/issues/11369>.
            loop {
                if !senders.excluded.contains(&addr) {
                    break;
                }
                addr = Address::random();
            }
            addr
        });

        if default_senders.is_empty() {
            random_sender.boxed()
        } else {
            proptest::prop_oneof![
                DEFAULT_SENDER_WEIGHT => any::<prop::sample::Index>()
                    .prop_map(move |index| *index.get(&default_senders)),
                RANDOM_SENDER_WEIGHT => random_sender,
            ]
            .boxed()
        }
    } else {
        any::<prop::sample::Index>().prop_map(move |index| *index.get(&senders.targeted)).boxed()
    }
}

fn default_invariant_senders(senders: &SenderFilters) -> Vec<Address> {
    DEFAULT_INVARIANT_SENDERS
        .into_iter()
        .filter(|sender| !senders.excluded.contains(sender))
        .collect()
}

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_contract_with_calldata<S: FuzzStateReader>(
    fuzz_state: &S,
    fuzz_fixtures: &FuzzFixtures,
    target: Address,
    func: Function,
    dictionary_weight: u32,
    payable_value_weight: u32,
) -> impl Strategy<Value = CallDetails> + use<S> {
    let is_payable = func.state_mutability == alloy_json_abi::StateMutability::Payable;
    let dictionary_weight = dictionary_weight.min(100);

    // We need to compose all the strategies generated for each parameter in all possible
    // combinations.
    // `prop_oneof!` / `TupleUnion` `Arc`s for cheap cloning.
    let calldata_strategy = prop_oneof![
        100 - dictionary_weight => fuzz_calldata(func.clone(), fuzz_fixtures),
        dictionary_weight => fuzz_calldata_from_state(func, fuzz_state, fuzz_fixtures),
    ];

    // For payable functions, generate random value using shared strategy.
    let value_strategy =
        if is_payable { fuzz_msg_value(payable_value_weight).boxed() } else { Just(None).boxed() };

    (calldata_strategy, value_strategy).prop_map(move |(calldata, value)| {
        trace!(input=?calldata, ?value);
        CallDetails { target, calldata, value }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sender_pool_includes_foundry_deployer() {
        let senders = SenderFilters::default();

        assert_eq!(
            default_invariant_senders(&senders),
            vec![
                address!("0x0000000000000000000000000000000000010000"),
                address!("0x0000000000000000000000000000000000020000"),
                CALLER,
            ]
        );
    }

    #[test]
    fn default_sender_pool_respects_exclusions() {
        let excluded = address!("0x0000000000000000000000000000000000010000");
        let senders = SenderFilters::new(vec![], vec![excluded, CALLER]);

        assert_eq!(
            default_invariant_senders(&senders),
            vec![address!("0x0000000000000000000000000000000000020000")]
        );
    }
}
