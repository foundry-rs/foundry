use crate::{strategies::fuzz_param, FuzzFixtures};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::Bytes;
use proptest::prelude::Strategy;

/// Given a function, it returns a strategy which generates valid calldata
/// for that function's input types, following declared test fixtures.
pub fn fuzz_calldata(func: Function, fuzz_fixtures: &FuzzFixtures) -> impl Strategy<Value = Bytes> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations, accounting any parameter declared fixture
    let strats = func
        .inputs
        .iter()
        .map(|input| {
            fuzz_param(
                &input.selector_type().parse().unwrap(),
                fuzz_fixtures.param_fixtures(&input.name),
            )
        })
        .collect::<Vec<_>>();
    strats.prop_map(move |values| {
        func.abi_encode_input(&values)
            .unwrap_or_else(|_| {
                panic!(
                    "Fuzzer generated invalid arguments for function `{}` with inputs {:?}: {:?}",
                    func.name, func.inputs, values
                )
            })
            .into()
    })
}
