use crate::fuzz::{
    fuzz_calldata, fuzz_calldata_from_state,
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts},
    strategies::fuzz_param,
    EvmFuzzState,
};
use ethers::{
    abi::{Abi, Function, ParamType},
    types::{Address, Bytes},
};
use parking_lot::RwLock;
use proptest::prelude::*;
pub use proptest::test_runner::Config as FuzzConfig;
use std::sync::Arc;

use super::fuzz_param_from_state;

/// Given a target address, we generate random calldata.
pub fn override_call_strat(
    fuzz_state: EvmFuzzState,
    contracts: FuzzRunIdentifiedContracts,
    target: Arc<RwLock<Address>>,
) -> SBoxedStrategy<(Address, Bytes)> {
    let contracts_ref = contracts.clone();

    let random_contract = any::<prop::sample::Selector>()
        .prop_map(move |selector| *selector.select(contracts_ref.lock().keys()));
    let target = any::<prop::sample::Selector>().prop_map(move |_| *target.read());

    proptest::strategy::Union::new_weighted(vec![
        (80, target.sboxed()),
        (20, random_contract.sboxed()),
    ])
    .prop_flat_map(move |target_address| {
        let fuzz_state = fuzz_state.clone();
        let (_, abi, functions) = contracts.lock().get(&target_address).unwrap().clone();

        let func = select_random_function(abi, functions);
        func.prop_flat_map(move |func| {
            fuzz_contract_with_calldata(fuzz_state.clone(), target_address, func)
        })
    })
    .sboxed()
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
    senders: Vec<Address>,
    contracts: FuzzRunIdentifiedContracts,
) -> BoxedStrategy<Vec<BasicTxDetails>> {
    // We only want to seed the first value, since we want to generate the rest as we mutate the
    // state
    vec![generate_call(fuzz_state, senders, contracts); 1].boxed()
}

/// Strategy to generate a transaction where the `sender`, `target` and `calldata` are all generated
/// through specific strategies.
fn generate_call(
    fuzz_state: EvmFuzzState,
    senders: Vec<Address>,
    contracts: FuzzRunIdentifiedContracts,
) -> BoxedStrategy<BasicTxDetails> {
    let random_contract = select_random_contract(contracts);
    random_contract
        .prop_flat_map(move |(contract, abi, functions)| {
            let func = select_random_function(abi, functions);
            let senders = senders.clone();
            let fuzz_state = fuzz_state.clone();
            func.prop_flat_map(move |func| {
                let sender = select_random_sender(fuzz_state.clone(), senders.clone());
                (sender, fuzz_contract_with_calldata(fuzz_state.clone(), contract, func))
            })
        })
        .boxed()
}

/// Strategy to select a sender address:
/// * If `senders` is empty, then it's either a random address (10%) or from the dictionary (90%).
/// * If `senders` is not empty, then there's an 80% chance that one from the list is selected. The
///   remaining 20% will either be a random address (10%) or from the dictionary (90%).
fn select_random_sender(
    fuzz_state: EvmFuzzState,
    senders: Vec<Address>,
) -> impl Strategy<Value = Address> {
    let fuzz_strategy = proptest::strategy::Union::new_weighted(vec![
        (
            10,
            fuzz_param(&ParamType::Address)
                .prop_map(move |addr| addr.into_address().unwrap())
                .boxed(),
        ),
        (
            90,
            fuzz_param_from_state(&ParamType::Address, fuzz_state)
                .prop_map(move |addr| addr.into_address().unwrap())
                .boxed(),
        ),
    ])
    .boxed();

    if !senders.is_empty() {
        let selector =
            any::<prop::sample::Selector>().prop_map(move |selector| *selector.select(&*senders));
        proptest::strategy::Union::new_weighted(vec![(80, selector.boxed()), (20, fuzz_strategy)])
            .boxed()
    } else {
        fuzz_strategy
    }
}

/// Strategy to randomly select a contract from the `contracts` list that has at least 1 function
fn select_random_contract(
    contracts: FuzzRunIdentifiedContracts,
) -> impl Strategy<Value = (Address, Abi, Vec<Function>)> {
    let selectors = any::<prop::sample::Selector>();

    selectors.prop_map(move |selector| {
        let contracts = contracts.lock();
        let (addr, (_, abi, functions)) =
            selector.select(contracts.iter().filter(|(_, (_, abi, _))| !abi.functions.is_empty()));
        (*addr, abi.clone(), functions.clone())
    })
}

/// Strategy to select a random mutable function from the abi.
///
/// If `targeted_functions` is not empty, select one from it. Otherwise, take any
/// of the available abi functions.
fn select_random_function(
    abi: Abi,
    targeted_functions: Vec<Function>,
) -> impl Strategy<Value = Function> {
    let selectors = any::<prop::sample::Selector>();
    let possible_funcs: Vec<ethers::abi::Function> = abi
        .functions()
        .filter(|func| {
            !matches!(
                func.state_mutability,
                ethers::abi::StateMutability::Pure | ethers::abi::StateMutability::View
            )
        })
        .cloned()
        .collect();

    let total_random = selectors.prop_map(move |selector| {
        let func = selector.select(&possible_funcs);
        func.clone()
    });

    if !targeted_functions.is_empty() {
        let selector = any::<prop::sample::Selector>()
            .prop_map(move |selector| selector.select(targeted_functions.clone()));

        selector.boxed()
    } else {
        total_random.boxed()
    }
}

/// Given a function, it returns a proptest strategy which generates valid abi-encoded calldata
/// for that function's input types.
pub fn fuzz_contract_with_calldata(
    fuzz_state: EvmFuzzState,
    contract: Address,
    func: Function,
) -> impl Strategy<Value = (Address, Bytes)> {
    // // We need to compose all the strategies generated for each parameter in all
    // // possible combinations
    let strats = proptest::strategy::Union::new_weighted(vec![
        (60, fuzz_calldata(func.clone())),
        (40, fuzz_calldata_from_state(func, fuzz_state)),
    ]);

    strats.prop_map(move |calldata| {
        tracing::trace!(input = ?calldata);
        (contract, calldata)
    })
}
