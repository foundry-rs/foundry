use super::state::EvmFuzzState;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, FixedBytes, I256, U256};
use arbitrary::Unstructured;
use proptest::prelude::*;

/// The max length of arrays we fuzz for is 256.
const MAX_ARRAY_LEN: usize = 256;

/// Given a parameter type, returns a strategy for generating values for that type.
///
/// Works with ABI Encoder v2 tuples.
pub fn fuzz_param(param: &DynSolType) -> BoxedStrategy<DynSolValue> {
    let param = param.to_owned();
    match param {
        DynSolType::Address => any::<[u8; 32]>()
            .prop_map(|x| DynSolValue::Address(Address::from_word(x.into())))
            .boxed(),
        DynSolType::Int(n) => {
            let strat = super::IntStrategy::new(n, vec![]);
            let strat = strat.prop_map(move |x| DynSolValue::Int(x, n));
            strat.boxed()
        }
        DynSolType::Uint(n) => {
            let strat = super::UintStrategy::new(n, vec![]);
            let strat = strat.prop_map(move |x| DynSolValue::Uint(x, n));
            strat.boxed()
        }
        DynSolType::Function | DynSolType::Bool | DynSolType::Bytes => {
            DynSolValue::type_strategy(&param).boxed()
        }
        DynSolType::FixedBytes(size) => prop::collection::vec(any::<u8>(), size)
            .prop_map(move |mut v| {
                v.reverse();
                while v.len() < 32 {
                    v.push(0);
                }
                DynSolValue::FixedBytes(FixedBytes::from_slice(&v), size)
            })
            .boxed(),
        DynSolType::String => DynSolValue::type_strategy(&param)
            .prop_map(move |value| {
                DynSolValue::String(
                    String::from_utf8_lossy(value.as_str().unwrap().as_bytes())
                        .trim()
                        .trim_end_matches('\0')
                        .to_string(),
                )
            })
            .boxed(),
        DynSolType::Tuple(params) => {
            params.iter().map(fuzz_param).collect::<Vec<_>>().prop_map(DynSolValue::Tuple).boxed()
        }
        DynSolType::FixedArray(param, size) => proptest::collection::vec(fuzz_param(&param), size)
            .prop_map(DynSolValue::FixedArray)
            .boxed(),
        DynSolType::Array(param) => proptest::collection::vec(fuzz_param(&param), 0..MAX_ARRAY_LEN)
            .prop_map(DynSolValue::Array)
            .boxed(),
        DynSolType::CustomStruct { .. } => panic!("unsupported type"),
    }
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
    let param = param.to_owned();

    // Convert the value based on the parameter type
    match param {
        DynSolType::Address => value
            .prop_map(move |value| DynSolValue::Address(Address::from_word(value.into())))
            .boxed(),
        DynSolType::FixedBytes(size) => value
            .prop_map(move |v| {
                let mut buf: [u8; 32] = [0; 32];

                for b in v[..size].iter().enumerate() {
                    buf[b.0] = *b.1
                }

                let mut unstructured_v = Unstructured::new(v.as_slice());
                DynSolValue::arbitrary_from_type(&param, &mut unstructured_v)
                    .unwrap_or(DynSolValue::FixedBytes(FixedBytes::from_slice(&buf), size))
            })
            .boxed(),
        DynSolType::Function | DynSolType::Bool => DynSolValue::type_strategy(&param).boxed(),
        DynSolType::String => DynSolValue::type_strategy(&param)
            .prop_map(move |value| {
                DynSolValue::String(
                    String::from_utf8_lossy(value.as_str().unwrap().as_bytes())
                        .trim()
                        .trim_end_matches('\0')
                        .to_string(),
                )
            })
            .boxed(),
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
                    DynSolValue::Int(num, y * 8)
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
                        U256::from_be_bytes(value) % U256::from(2).pow(U256::from(y * 8)),
                        y * 8,
                    )
                })
                .boxed(),
            _ => panic!("unsupported solidity type uint{n}"),
        },
        DynSolType::Tuple(params) => params
            .iter()
            .map(|p| fuzz_param_from_state(p, arc_state.clone()))
            .collect::<Vec<_>>()
            .prop_map(DynSolValue::Tuple)
            .boxed(),
        DynSolType::Bytes => value.prop_map(move |value| DynSolValue::Bytes(value.into())).boxed(),
        DynSolType::FixedArray(param, size) => {
            let fixed_size = size;
            proptest::collection::vec(fuzz_param_from_state(&param, arc_state), fixed_size)
                .prop_map(DynSolValue::FixedArray)
                .boxed()
        }
        DynSolType::Array(param) => {
            proptest::collection::vec(fuzz_param_from_state(&param, arc_state), 0..MAX_ARRAY_LEN)
                .prop_map(DynSolValue::Array)
                .boxed()
        }
        DynSolType::CustomStruct { .. } => panic!("unsupported type"),
    }
}

#[cfg(test)]
mod tests {
    use crate::strategies::{build_initial_state, fuzz_calldata, fuzz_calldata_from_state};
    use alloy_json_abi::Function;
    use foundry_config::FuzzDictionaryConfig;
    use revm::db::{CacheDB, EmptyDB};

    #[test]
    fn can_fuzz_array() {
        let f = "testArray(uint64[2] calldata values)";
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
