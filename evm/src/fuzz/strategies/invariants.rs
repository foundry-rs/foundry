use parking_lot::RwLock;
use std::sync::Arc;

use ethers::{
    abi::{Abi, Function, ParamType},
    types::{Address, Bytes},
};
use proptest::prelude::*;
pub use proptest::test_runner::Config as FuzzConfig;

use crate::fuzz::{
    fuzz_calldata, fuzz_calldata_from_state,
    invariant::{BasicTxDetails, FuzzRunIdentifiedContracts},
    strategies::fuzz_param,
    EvmFuzzState,
};

/// Given a target address, we generate random calldata.
pub fn reentrancy_strat(
    fuzz_state: EvmFuzzState,
    contracts: FuzzRunIdentifiedContracts,
    target: Arc<RwLock<Address>>,
) -> SBoxedStrategy<(Address, Bytes)> {
    any::<prop::sample::Selector>()
        .prop_flat_map(move |_| {
            let fuzz_state = fuzz_state.clone();
            let target_address = *target.read();
            let contracts = contracts.read();
            let (_, abi, functions) = contracts.get(&target_address).unwrap();

            let func = select_random_function(abi.clone(), functions.clone());
            func.prop_flat_map(move |func| {
                fuzz_contract_with_calldata(fuzz_state.clone(), target_address, func)
            })
        })
        .sboxed()
}

pub fn invariant_strat(
    fuzz_state: EvmFuzzState,
    senders: Vec<Address>,
    contracts: FuzzRunIdentifiedContracts,
) -> SBoxedStrategy<Vec<BasicTxDetails>> {
    // We only want to seed the first value, since we want to generate the rest as we mutate the
    // state
    vec![generate_call(fuzz_state, senders, contracts); 1].sboxed()
}

fn generate_call(
    fuzz_state: EvmFuzzState,
    senders: Vec<Address>,
    contracts: FuzzRunIdentifiedContracts,
) -> SBoxedStrategy<BasicTxDetails> {
    let random_contract = select_random_contract(contracts);
    random_contract
        .prop_flat_map(move |(contract, abi, functions)| {
            let func = select_random_function(abi, functions);
            let senders = senders.clone();
            let fuzz_state = fuzz_state.clone();
            func.prop_flat_map(move |func| {
                let sender = select_random_sender(senders.clone());
                (sender, fuzz_contract_with_calldata(fuzz_state.clone(), contract, func))
            })
        })
        .sboxed()
}

fn select_random_sender(senders: Vec<Address>) -> impl Strategy<Value = Address> {
    let fuzz_strategy =
        fuzz_param(&ParamType::Address).prop_map(move |addr| addr.into_address().unwrap()).sboxed();

    if !senders.is_empty() {
        let selector =
            any::<prop::sample::Selector>().prop_map(move |selector| *selector.select(&*senders));
        proptest::strategy::Union::new_weighted(vec![(80, selector.sboxed()), (20, fuzz_strategy)])
            .sboxed()
    } else {
        fuzz_strategy
    }
}

fn select_random_contract(
    contracts: FuzzRunIdentifiedContracts,
) -> impl Strategy<Value = (Address, Abi, Vec<Function>)> {
    let selectors = any::<prop::sample::Selector>();

    selectors.prop_map(move |selector| {
        let contracts = contracts.read();
        let (addr, (_, abi, functions)) = selector.select(contracts.iter());
        (*addr, abi.clone(), functions.clone())
    })
}

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

        selector.sboxed()
        // todo make it an union too?
        // proptest::strategy::Union::new_weighted(vec![
        //     (100, selector.sboxed()),
        //     (0, total_random.sboxed()),
        // ])
        // .sboxed()
    } else {
        total_random.sboxed()
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
