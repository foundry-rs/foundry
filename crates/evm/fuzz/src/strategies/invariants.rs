use super::{fuzz_calldata, fuzz_param_from_state};
use crate::{
    BasicTxDetails, CallDetails, FuzzFixtures,
    invariant::{FuzzRunIdentifiedContracts, SenderFilters},
    strategies::{EvmFuzzState, fuzz_calldata_from_state, fuzz_param},
};
use alloy_json_abi::Function;
use alloy_primitives::{Address, U256};
use foundry_config::InvariantConfig;
use parking_lot::RwLock;
use proptest::prelude::*;
use rand::seq::IteratorRandom;
use std::{rc::Rc, sync::Arc};

/// Given a target address, we generate random calldata.
pub fn override_call_strat(
    fuzz_state: EvmFuzzState,
    contracts: FuzzRunIdentifiedContracts,
    target: Arc<RwLock<Address>>,
    fuzz_fixtures: FuzzFixtures,
) -> impl Strategy<Value = CallDetails> + Send + Sync + 'static {
    let contracts_ref = contracts.targets.clone();
    proptest::prop_oneof![
        80 => proptest::strategy::LazyJust::new(move || *target.read()),
        20 => any::<prop::sample::Selector>()
            .prop_map(move |selector| *selector.select(contracts_ref.lock().keys())),
    ]
    .prop_flat_map(move |target_address| {
        let fuzz_state = fuzz_state.clone();
        let fuzz_fixtures = fuzz_fixtures.clone();

        let func = {
            let contracts = contracts.targets.lock();
            let contract = contracts.get(&target_address).unwrap_or_else(|| {
                // Choose a random contract if target selected by lazy strategy is not in fuzz run
                // identified contracts. This can happen when contract is created in `setUp` call
                // but is not included in targetContracts.
                contracts.values().choose(&mut rand::rng()).unwrap()
            });
            let fuzzed_functions: Vec<_> = contract.abi_fuzzed_functions().cloned().collect();
            any::<prop::sample::Index>().prop_map(move |index| index.get(&fuzzed_functions).clone())
        };

        func.prop_flat_map(move |func| {
            fuzz_contract_with_calldata(&fuzz_state, &fuzz_fixtures, target_address, func)
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
    fuzz_state: EvmFuzzState,
    senders: SenderFilters,
    contracts: FuzzRunIdentifiedContracts,
    config: InvariantConfig,
    fuzz_fixtures: FuzzFixtures,
) -> impl Strategy<Value = BasicTxDetails> {
    let senders = Rc::new(senders);
    let dictionary_weight = config.dictionary.dictionary_weight;

    // Strategy to generate values for tx warp and roll.
    let warp_roll_strat = |cond: bool| {
        if cond { any::<U256>().prop_map(Some).boxed() } else { Just(None).boxed() }
    };

    any::<prop::sample::Selector>()
        .prop_flat_map(move |selector| {
            let contracts = contracts.targets.lock();
            let functions = contracts.fuzzed_functions();
            let (target_address, target_function) = selector.select(functions);

            let sender = select_random_sender(&fuzz_state, senders.clone(), dictionary_weight);

            let call_details = fuzz_contract_with_calldata(
                &fuzz_state,
                &fuzz_fixtures,
                *target_address,
                target_function.clone(),
            );

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
/// * If `senders` is empty, then it's either a random address (10%) or from the dictionary (90%).
/// * If `senders` is not empty, a random address is chosen from the list of senders.
fn select_random_sender(
    fuzz_state: &EvmFuzzState,
    senders: Rc<SenderFilters>,
    dictionary_weight: u32,
) -> impl Strategy<Value = Address> + use<> {
    if !senders.targeted.is_empty() {
        any::<prop::sample::Index>().prop_map(move |index| *index.get(&senders.targeted)).boxed()
    } else {
        assert!(dictionary_weight <= 100, "dictionary_weight must be <= 100");
        proptest::prop_oneof![
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
        })
        .boxed()
    }
}

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_contract_with_calldata(
    fuzz_state: &EvmFuzzState,
    fuzz_fixtures: &FuzzFixtures,
    target: Address,
    func: Function,
) -> impl Strategy<Value = CallDetails> + use<> {
    // We need to compose all the strategies generated for each parameter in all possible
    // combinations.
    // `prop_oneof!` / `TupleUnion` `Arc`s for cheap cloning.
    prop_oneof![
        60 => fuzz_calldata(func.clone(), fuzz_fixtures),
        40 => fuzz_calldata_from_state(func, fuzz_state),
    ]
    .prop_map(move |calldata| {
        trace!(input=?calldata);
        CallDetails { target, calldata }
    })
}
