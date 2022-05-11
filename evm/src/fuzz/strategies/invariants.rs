use ethers::{
    abi::{Abi, Function, ParamType},
    types::{Address, Bytes},
};
use std::collections::BTreeMap;

use proptest::prelude::*;
pub use proptest::test_runner::Config as FuzzConfig;

use crate::fuzz::{fuzz_calldata, fuzz_calldata_from_state, strategies::fuzz_param, EvmFuzzState};

// use super::{};

pub fn invariant_strat(
    fuzz_state: EvmFuzzState,
    depth: usize,
    senders: Vec<Address>,
    contracts: BTreeMap<Address, (String, Abi)>,
) -> BoxedStrategy<Vec<(Address, (Address, Bytes))>> {
    let iters = 1..depth + 1;
    proptest::collection::vec(gen_call(fuzz_state, senders, contracts), iters).boxed()
}

fn gen_call(
    fuzz_state: EvmFuzzState,
    senders: Vec<Address>,
    contracts: BTreeMap<Address, (String, Abi)>,
) -> BoxedStrategy<(Address, (Address, Bytes))> {
    let random_contract = select_random_contract(contracts);
    random_contract
        .prop_flat_map(move |(contract, abi)| {
            let func = select_random_function(abi);
            let senders = senders.clone();
            let fuzz_state = fuzz_state.clone();
            func.prop_flat_map(move |func| {
                let sender = select_random_sender(senders.clone());
                (sender, fuzz_contract_with_calldata(fuzz_state.clone(), contract, func))
            })
        })
        .boxed()
}

fn select_random_sender(senders: Vec<Address>) -> impl Strategy<Value = Address> {
    let fuzz_strategy =
        fuzz_param(&ParamType::Address).prop_map(move |addr| addr.into_address().unwrap()).boxed();

    if !senders.is_empty() {
        let selector =
            any::<prop::sample::Selector>().prop_map(move |selector| *selector.select(&senders));
        proptest::strategy::Union::new_weighted(vec![(80, selector.boxed()), (20, fuzz_strategy)])
            .boxed()
    } else {
        fuzz_strategy
    }
}

fn select_random_contract(
    contracts: BTreeMap<Address, (String, Abi)>,
) -> impl Strategy<Value = (Address, Abi)> {
    let selectors = any::<prop::sample::Selector>();
    selectors.prop_map(move |selector| {
        let res = selector.select(&contracts);
        (*res.0, res.1 .1.clone())
    })
}

fn select_random_function(abi: Abi) -> impl Strategy<Value = Function> {
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
    selectors.prop_map(move |selector| {
        let func = selector.select(&possible_funcs);
        func.clone()
    })
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
