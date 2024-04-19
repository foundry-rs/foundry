use super::state::EvmFuzzState;
use crate::strategies::calldata::CalldataFuzzDictionary;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, B256, I256, U256};
use proptest::prelude::*;

/// The max length of arrays we fuzz for is 256.
const MAX_ARRAY_LEN: usize = 256;

/// Given a parameter type, returns a strategy for generating values for that type.
///
/// Works with ABI Encoder v2 tuples.
pub fn fuzz_param(
    param: &DynSolType,
    config: Option<&CalldataFuzzDictionary>,
) -> BoxedStrategy<DynSolValue> {
    match *param {
        DynSolType::Address => {
            if let Some(config) = config {
                let len = config.inner.addresses.len();
                if len > 0 {
                    let dict = config.inner.clone();
                    // Create strategy to return random address from configured dictionary.
                    return any::<prop::sample::Index>()
                        .prop_map(move |index| {
                            let index = index.index(len);
                            DynSolValue::Address(dict.addresses[index])
                        })
                        .boxed();
                }
            }

            // If no config for addresses dictionary then create unbounded addresses strategy.
            any::<Address>().prop_map(DynSolValue::Address).boxed()
        }
        DynSolType::Int(n @ 8..=256) => {
            super::IntStrategy::new(n, vec![]).prop_map(move |x| DynSolValue::Int(x, n)).boxed()
        }
        DynSolType::Uint(n @ 8..=256) => {
            super::UintStrategy::new(n, vec![]).prop_map(move |x| DynSolValue::Uint(x, n)).boxed()
        }
        DynSolType::Function | DynSolType::Bool | DynSolType::Bytes => {
            DynSolValue::type_strategy(param).boxed()
        }
        DynSolType::FixedBytes(size @ 1..=32) => any::<B256>()
            .prop_map(move |mut v| {
                v[size..].fill(0);
                DynSolValue::FixedBytes(v, size)
            })
            .boxed(),
        DynSolType::String => DynSolValue::type_strategy(param)
            .prop_map(move |value| {
                DynSolValue::String(
                    value.as_str().unwrap().trim().trim_end_matches('\0').to_string(),
                )
            })
            .boxed(),

        DynSolType::Tuple(ref params) => params
            .iter()
            .map(|p| fuzz_param(p, config))
            .collect::<Vec<_>>()
            .prop_map(DynSolValue::Tuple)
            .boxed(),
        DynSolType::FixedArray(ref param, size) => {
            proptest::collection::vec(fuzz_param(param, config), size)
                .prop_map(DynSolValue::FixedArray)
                .boxed()
        }
        DynSolType::Array(ref param) => {
            proptest::collection::vec(fuzz_param(param, config), 0..MAX_ARRAY_LEN)
                .prop_map(DynSolValue::Array)
                .boxed()
        }
        _ => panic!("unsupported fuzz param type: {param}"),
    }
}

/// Given a parameter type, returns a strategy for generating values for that type, given some EVM
/// fuzz state.
///
/// Works with ABI Encoder v2 tuples.
pub fn fuzz_param_from_state(
    param: &DynSolType,
    state: &EvmFuzzState,
) -> BoxedStrategy<DynSolValue> {
    // Value strategy that uses the state.
    let value = || {
        let state = state.clone();
        let param = param.clone();
        // Use `Index` instead of `Selector` to not iterate over the entire dictionary.
        any::<prop::sample::Index>().prop_map(move |index| {
            let state = state.dictionary_read();
            let bias = rand::thread_rng().gen_range(0..100);
            let values = match bias {
                x if x < 50 => {
                    if let Some(sample_values) = state.samples(param.clone()) {
                        sample_values
                    } else {
                        state.values()
                    }
                }
                _ => state.values(),
            };
            let index = index.index(values.len());
            *values.iter().nth(index).unwrap()
        })
    };

    // Convert the value based on the parameter type
    match *param {
        DynSolType::Address => value()
            .prop_map(move |value| DynSolValue::Address(Address::from_word(value.into())))
            .boxed(),
        DynSolType::Function => value()
            .prop_map(move |value| {
                DynSolValue::Function(alloy_primitives::Function::from_word(value.into()))
            })
            .boxed(),
        DynSolType::FixedBytes(size @ 1..=32) => value()
            .prop_map(move |mut v| {
                v[size..].fill(0);
                DynSolValue::FixedBytes(B256::from(v), size)
            })
            .boxed(),
        DynSolType::Bool => DynSolValue::type_strategy(param).boxed(),
        DynSolType::String => DynSolValue::type_strategy(param)
            .prop_map(move |value| {
                DynSolValue::String(
                    value.as_str().unwrap().trim().trim_end_matches('\0').to_string(),
                )
            })
            .boxed(),
        DynSolType::Bytes => {
            value().prop_map(move |value| DynSolValue::Bytes(value.into())).boxed()
        }
        DynSolType::Int(n @ 8..=256) => match n / 8 {
            32 => value()
                .prop_map(move |value| {
                    DynSolValue::Int(I256::from_raw(U256::from_be_bytes(value)), 256)
                })
                .boxed(),
            1..=31 => value()
                .prop_map(move |value| {
                    // Generate a uintN in the correct range, then shift it to the range of intN
                    // by subtracting 2^(N-1)
                    let uint = U256::from_be_bytes(value) % U256::from(1).wrapping_shl(n);
                    let max_int_plus1 = U256::from(1).wrapping_shl(n - 1);
                    let num = I256::from_raw(uint.wrapping_sub(max_int_plus1));
                    DynSolValue::Int(num, n)
                })
                .boxed(),
            _ => unreachable!(),
        },
        DynSolType::Uint(n @ 8..=256) => match n / 8 {
            32 => value()
                .prop_map(move |value| DynSolValue::Uint(U256::from_be_bytes(value), 256))
                .boxed(),
            1..=31 => value()
                .prop_map(move |value| {
                    DynSolValue::Uint(U256::from_be_bytes(value) % U256::from(1).wrapping_shl(n), n)
                })
                .boxed(),
            _ => unreachable!(),
        },
        DynSolType::Tuple(ref params) => params
            .iter()
            .map(|p| fuzz_param_from_state(p, state))
            .collect::<Vec<_>>()
            .prop_map(DynSolValue::Tuple)
            .boxed(),
        DynSolType::FixedArray(ref param, size) => {
            proptest::collection::vec(fuzz_param_from_state(param, state), size)
                .prop_map(DynSolValue::FixedArray)
                .boxed()
        }
        DynSolType::Array(ref param) => {
            proptest::collection::vec(fuzz_param_from_state(param, state), 0..MAX_ARRAY_LEN)
                .prop_map(DynSolValue::Array)
                .boxed()
        }
        _ => panic!("unsupported fuzz param type: {param}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::strategies::{build_initial_state, fuzz_calldata, fuzz_calldata_from_state};
    use foundry_common::abi::get_func;
    use foundry_config::FuzzDictionaryConfig;
    use revm::db::{CacheDB, EmptyDB};

    #[test]
    fn can_fuzz_array() {
        let f = "testArray(uint64[2] calldata values)";
        let func = get_func(f).unwrap();
        let db = CacheDB::new(EmptyDB::default());
        let state = build_initial_state(&db, FuzzDictionaryConfig::default());
        let strat = proptest::prop_oneof![
            60 => fuzz_calldata(func.clone()),
            40 => fuzz_calldata_from_state(func, &state),
        ];
        let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
        let mut runner = proptest::test_runner::TestRunner::new(cfg);
        let _ = runner.run(&strat, |_| Ok(()));
    }
}
