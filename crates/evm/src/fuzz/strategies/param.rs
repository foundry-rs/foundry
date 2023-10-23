use super::state::EvmFuzzState;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, FixedBytes, I256, U256};
use proptest::prelude::*;

/// The max length of arrays we fuzz for is 256.
pub const MAX_ARRAY_LEN: usize = 256;
/// Given a parameter type, returns a strategy for generating values for that type.
///
/// Works with ABI Encoder v2 tuples.
pub fn fuzz_param(param: &DynSolType) -> SBoxedStrategy<DynSolValue> {
    DynSolValue::type_strategy(param)
}

/// Given a parameter type, returns a strategy for generating values for that type, given some EVM
/// fuzz state.
///
/// Works with ABI Encoder v2 tuples.
pub fn fuzz_param_from_state(
    param: &DynSolType,
    arc_state: EvmFuzzState,
) -> BoxedStrategy<DynSolValue> {
    // These are to comply with lifetime requirements
    let state_len = arc_state.read().values().len();

    // Select a value from the state
    let st = arc_state.clone();
    let value = any::<prop::sample::Index>()
        .prop_map(move |index| index.index(state_len))
        .prop_map(move |index| *st.read().values().iter().nth(index).unwrap());

    // Convert the value based on the parameter type
    match param {
        DynSolType::Address => value
            .prop_map(move |value| DynSolValue::Address(Address::from_word(value.into())))
            .boxed(),
        DynSolType::Bytes => value.prop_map(move |value| DynSolValue::Bytes(value.into())).boxed(),
        DynSolType::Int(n) => match n / 8 {
            32 => value
                .prop_map(move |value| {
                    DynSolValue::Int(I256::from_raw(U256::from_be_bytes(value)), 256)
                })
                .boxed(),
            y @ 1..=31 => value
                .prop_map(move |value| {
                    // Generate a uintN in the correct range, then shift it to the range of intN
                    // by subtracting 2^(N-1)
                    let uint =
                        U256::from_be_bytes(value) % U256::from(2usize).pow(U256::from(y * 8));
                    let max_int_plus1 = U256::from(2usize).pow(U256::from(y * 8 - 1));
                    let num = I256::from_raw(uint.overflowing_sub(max_int_plus1).0);
                    DynSolValue::Int(num, 256)
                })
                .boxed(),
            _ => panic!("unsupported solidity type int{n}"),
        },
        DynSolType::Uint(n) => match n / 8 {
            32 => value
                .prop_map(move |value| DynSolValue::Uint(U256::from_be_bytes(value), 256))
                .boxed(),
            y @ 1..=31 => value
                .prop_map(move |value| {
                    DynSolValue::Uint(
                        U256::from_be_bytes(value) % (U256::from(2usize).pow(U256::from(y * 8))),
                        y * 8,
                    )
                })
                .boxed(),
            _ => panic!("unsupported solidity type uint{n}"),
        },
        DynSolType::Bool => value.prop_map(move |value| DynSolValue::Bool(value[31] == 1)).boxed(),
        DynSolType::String => value
            .prop_map(move |value| {
                DynSolValue::String(
                    String::from_utf8_lossy(&value[..]).trim().trim_end_matches('\0').to_string(),
                )
            })
            .boxed(),
        DynSolType::Array(param) => {
            proptest::collection::vec(fuzz_param_from_state(param, arc_state), 0..MAX_ARRAY_LEN)
                .prop_map(DynSolValue::Array)
                .boxed()
        }
        DynSolType::FixedBytes(size) => {
            let size = *size;
            value
                .prop_map(move |value| {
                    DynSolValue::FixedBytes(FixedBytes::from_slice(&value[32 - size..]), size)
                })
                .boxed()
        }
        DynSolType::FixedArray(param, size) => {
            let fixed_size = *size;
            proptest::collection::vec(fuzz_param_from_state(param, arc_state), fixed_size)
                .prop_map(DynSolValue::FixedArray)
                .boxed()
        }
        DynSolType::Tuple(params) => params
            .iter()
            .map(|p| fuzz_param_from_state(p, arc_state.clone()))
            .collect::<Vec<_>>()
            .prop_map(DynSolValue::Tuple)
            .boxed(),
        _ => unimplemented!(),
    }
}

#[cfg(test)]
mod tests {
    use crate::fuzz::strategies::{build_initial_state, fuzz_calldata, fuzz_calldata_from_state};
    use alloy_json_abi::Function;
    use foundry_config::FuzzDictionaryConfig;
    use revm::db::{CacheDB, EmptyDB};

    #[test]
    fn can_fuzz_array() {
        let f = "function testArray(uint64[2] calldata values)";
        let func = Function::parse(f).unwrap();
        let db = CacheDB::new(EmptyDB::default());
        let state = build_initial_state(&db, &FuzzDictionaryConfig::default());
        let strat = proptest::strategy::Union::new_weighted(vec![
            (60, fuzz_calldata(func.clone())),
            (40, fuzz_calldata_from_state(func, state)),
        ]);
        let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
        let mut runner = proptest::test_runner::TestRunner::new(cfg);
        let _ = runner.run(&strat, |_| Ok(()));
    }
}
