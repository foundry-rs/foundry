use crate::{
    strategies::{fuzz_param_from_state, fuzz_param_with_fixtures, EvmFuzzState},
    FuzzFixtures,
};
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
            fuzz_param_with_fixtures(
                &input.selector_type().parse().unwrap(),
                fuzz_fixtures.param_fixtures(&input.name),
                &input.name,
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

/// Given a function and some state, it returns a strategy which generated valid calldata for the
/// given function's input types, based on state taken from the EVM.
pub fn fuzz_calldata_from_state(
    func: Function,
    state: &EvmFuzzState,
) -> impl Strategy<Value = Bytes> {
    let strats = func
        .inputs
        .iter()
        .map(|input| fuzz_param_from_state(&input.selector_type().parse().unwrap(), state))
        .collect::<Vec<_>>();
    strats
        .prop_map(move |values| {
            func.abi_encode_input(&values)
                .unwrap_or_else(|_| {
                    panic!(
                        "Fuzzer generated invalid arguments for function `{}` with inputs {:?}: {:?}",
                        func.name, func.inputs, values
                    )
                })
                .into()
        })
        .no_shrink()
}

#[cfg(test)]
mod tests {
    use crate::{strategies::fuzz_calldata, FuzzFixtures};
    use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
    use alloy_json_abi::Function;
    use alloy_primitives::Address;
    use proptest::prelude::Strategy;
    use std::collections::HashMap;

    #[test]
    fn can_fuzz_with_fixtures() {
        let function = Function::parse("test_fuzzed_address(address addressFixture)").unwrap();

        let address_fixture = DynSolValue::Address(Address::random());
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "addressFixture".to_string(),
            DynSolValue::Array(vec![address_fixture.clone()]),
        );

        let expected = function.abi_encode_input(&[address_fixture]).unwrap();
        let strategy = fuzz_calldata(function, &FuzzFixtures::new(fixtures));
        let _ = strategy.prop_map(move |fuzzed| {
            assert_eq!(expected, fuzzed);
        });
    }
}
