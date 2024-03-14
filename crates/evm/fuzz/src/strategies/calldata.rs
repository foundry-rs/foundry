use crate::{strategies::fuzz_param, FuzzFixtures};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::Bytes;
use proptest::prelude::{BoxedStrategy, Strategy};

/// Given a function, it returns a strategy which generates valid calldata
/// for that function's input types, following declared test fixtures.
pub fn fuzz_calldata(func: Function, fuzz_fixtures: FuzzFixtures) -> BoxedStrategy<Bytes> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
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

    strats
        .prop_map(move |tokens| {
            trace!(input=?tokens);
            func.abi_encode_input(&tokens).unwrap().into()
        })
        .boxed()
}
