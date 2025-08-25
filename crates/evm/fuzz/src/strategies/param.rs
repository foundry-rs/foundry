use super::state::EvmFuzzState;
use crate::strategies::mutators::{AbiMutator, IncrementDecrementMutator};
use alloy_dyn_abi::{DynSolType, DynSolValue, Word};
use alloy_primitives::{Address, B256, I256, U256};
use proptest::{prelude::*, test_runner::TestRunner};
use rand::{SeedableRng, prelude::IndexedMutRandom, rngs::StdRng};
use std::mem::replace;

/// The max length of arrays we fuzz for is 256.
const MAX_ARRAY_LEN: usize = 256;

/// Given a parameter type, returns a strategy for generating values for that type.
///
/// See [`fuzz_param_with_fixtures`] for more information.
pub fn fuzz_param(param: &DynSolType) -> BoxedStrategy<DynSolValue> {
    fuzz_param_inner(param, None)
}

/// Given a parameter type and configured fixtures for param name, returns a strategy for generating
/// values for that type.
///
/// Fixtures can be currently generated for uint, int, address, bytes and
/// string types and are defined for parameter name.
/// For example, fixtures for parameter `owner` of type `address` can be defined in a function with
/// a `function fixture_owner() public returns (address[] memory)` signature.
///
/// Fixtures are matched on parameter name, hence fixtures defined in
/// `fixture_owner` function can be used in a fuzzed test function with a signature like
/// `function testFuzz_ownerAddress(address owner, uint amount)`.
///
/// Raises an error if all the fixture types are not of the same type as the input parameter.
///
/// Works with ABI Encoder v2 tuples.
pub fn fuzz_param_with_fixtures(
    param: &DynSolType,
    fixtures: Option<&[DynSolValue]>,
    name: &str,
) -> BoxedStrategy<DynSolValue> {
    fuzz_param_inner(param, fixtures.map(|f| (f, name)))
}

fn fuzz_param_inner(
    param: &DynSolType,
    mut fuzz_fixtures: Option<(&[DynSolValue], &str)>,
) -> BoxedStrategy<DynSolValue> {
    if let Some((fixtures, name)) = fuzz_fixtures
        && !fixtures.iter().all(|f| f.matches(param))
    {
        error!("fixtures for {name:?} do not match type {param}");
        fuzz_fixtures = None;
    }
    let fuzz_fixtures = fuzz_fixtures.map(|(f, _)| f);

    let value = || {
        let default_strategy = DynSolValue::type_strategy(param);
        if let Some(fixtures) = fuzz_fixtures {
            proptest::prop_oneof![
                50 => {
                    let fixtures = fixtures.to_vec();
                    any::<prop::sample::Index>()
                        .prop_map(move |index| index.get(&fixtures).clone())
                },
                50 => default_strategy,
            ]
            .boxed()
        } else {
            default_strategy.boxed()
        }
    };

    match *param {
        DynSolType::Address => value(),
        DynSolType::Int(n @ 8..=256) => super::IntStrategy::new(n, fuzz_fixtures)
            .prop_map(move |x| DynSolValue::Int(x, n))
            .boxed(),
        DynSolType::Uint(n @ 8..=256) => super::UintStrategy::new(n, fuzz_fixtures)
            .prop_map(move |x| DynSolValue::Uint(x, n))
            .boxed(),
        DynSolType::Function | DynSolType::Bool => DynSolValue::type_strategy(param).boxed(),
        DynSolType::Bytes => value(),
        DynSolType::FixedBytes(_size @ 1..=32) => value(),
        DynSolType::String => value()
            .prop_map(move |value| {
                DynSolValue::String(
                    value.as_str().unwrap().trim().trim_end_matches('\0').to_string(),
                )
            })
            .boxed(),
        DynSolType::Tuple(ref params) => params
            .iter()
            .map(|param| fuzz_param_inner(param, None))
            .collect::<Vec<_>>()
            .prop_map(DynSolValue::Tuple)
            .boxed(),
        DynSolType::FixedArray(ref param, size) => {
            proptest::collection::vec(fuzz_param_inner(param, None), size)
                .prop_map(DynSolValue::FixedArray)
                .boxed()
        }
        DynSolType::Array(ref param) => {
            proptest::collection::vec(fuzz_param_inner(param, None), 0..MAX_ARRAY_LEN)
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
        // Generate a bias and use it to pick samples or non-persistent values (50 / 50).
        // Use `Index` instead of `Selector` when selecting a value to avoid iterating over the
        // entire dictionary.
        any::<(bool, prop::sample::Index)>().prop_map(move |(bias, index)| {
            let state = state.dictionary_read();
            let values = if bias { state.samples(&param) } else { None }
                .unwrap_or_else(|| state.values())
                .as_slice();
            values[index.index(values.len())]
        })
    };

    // Convert the value based on the parameter type
    match *param {
        DynSolType::Address => {
            let deployed_libs = state.deployed_libs.clone();
            value()
                .prop_map(move |value| {
                    let mut fuzzed_addr = Address::from_word(value);
                    if deployed_libs.contains(&fuzzed_addr) {
                        let mut rng = StdRng::seed_from_u64(0x1337); // use deterministic rng

                        // Do not use addresses of deployed libraries as fuzz input, instead return
                        // a deterministically random address. We cannot filter out this value (via
                        // `prop_filter_map`) as proptest can invoke this closure after test
                        // execution, and returning a `None` will cause it to panic.
                        // See <https://github.com/foundry-rs/foundry/issues/9764> and <https://github.com/foundry-rs/foundry/issues/8639>.
                        loop {
                            fuzzed_addr.randomize_with(&mut rng);
                            if !deployed_libs.contains(&fuzzed_addr) {
                                break;
                            }
                        }
                    }
                    DynSolValue::Address(fuzzed_addr)
                })
                .boxed()
        }
        DynSolType::Function => value()
            .prop_map(move |value| {
                DynSolValue::Function(alloy_primitives::Function::from_word(value))
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
            value().prop_map(move |value| DynSolValue::Bytes(value.0.into())).boxed()
        }
        DynSolType::Int(n @ 8..=256) => match n / 8 {
            32 => value()
                .prop_map(move |value| DynSolValue::Int(I256::from_raw(value.into()), 256))
                .boxed(),
            1..=31 => value()
                .prop_map(move |value| {
                    // Generate a uintN in the correct range, then shift it to the range of intN
                    // by subtracting 2^(N-1)
                    let uint = U256::from_be_bytes(value.0) % U256::from(1).wrapping_shl(n);
                    let max_int_plus1 = U256::from(1).wrapping_shl(n - 1);
                    let num = I256::from_raw(uint.wrapping_sub(max_int_plus1));
                    DynSolValue::Int(num, n)
                })
                .boxed(),
            _ => unreachable!(),
        },
        DynSolType::Uint(n @ 8..=256) => match n / 8 {
            32 => value()
                .prop_map(move |value| DynSolValue::Uint(U256::from_be_bytes(value.0), 256))
                .boxed(),
            1..=31 => value()
                .prop_map(move |value| {
                    let uint = U256::from_be_bytes(value.0) % U256::from(1).wrapping_shl(n);
                    DynSolValue::Uint(uint, n)
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

/// Mutates the current value of the given parameter type and value.
pub fn mutate_param_value(
    param: &DynSolType,
    value: DynSolValue,
    test_runner: &mut TestRunner,
    state: &EvmFuzzState,
) -> DynSolValue {
    let new_value = |param: &DynSolType, test_runner: &mut TestRunner| {
        fuzz_param_from_state(param, state)
            .new_tree(test_runner)
            .expect("Could not generate case")
            .current()
    };

    match value {
        DynSolValue::Bool(val) => {
            // flip boolean value
            trace!(target: "mutator", "Bool flip {val}");
            Some(DynSolValue::Bool(!val))
        }
        DynSolValue::Uint(val, size) => match test_runner.rng().random_range(0..=5) {
            0 => U256::increment_decrement(val, size, test_runner),
            1 => U256::flip_random_bit(val, size, test_runner),
            2 => U256::mutate_interesting_byte(val, size, test_runner),
            3 => U256::mutate_interesting_word(val, size, test_runner),
            4 => U256::mutate_interesting_dword(val, size, test_runner),
            5 => None,
            _ => unreachable!(),
        }
        .map(|v| DynSolValue::Uint(v, size)),
        DynSolValue::Int(val, size) => match test_runner.rng().random_range(0..=5) {
            0 => I256::increment_decrement(val, size, test_runner),
            1 => I256::flip_random_bit(val, size, test_runner),
            2 => I256::mutate_interesting_byte(val, size, test_runner),
            3 => I256::mutate_interesting_word(val, size, test_runner),
            4 => I256::mutate_interesting_dword(val, size, test_runner),
            5 => None,
            _ => unreachable!(),
        }
        .map(|v| DynSolValue::Int(v, size)),
        DynSolValue::Address(val) => match test_runner.rng().random_range(0..=4) {
            0 => Address::flip_random_bit(val, 20, test_runner),
            1 => Address::mutate_interesting_byte(val, 20, test_runner),
            2 => Address::mutate_interesting_word(val, 20, test_runner),
            3 => Address::mutate_interesting_dword(val, 20, test_runner),
            4 => None,
            _ => unreachable!(),
        }
        .map(DynSolValue::Address),
        DynSolValue::Array(mut values) => {
            if let DynSolType::Array(param_type) = param
                && !values.is_empty()
            {
                match test_runner.rng().random_range(0..=2) {
                    // Decrease array size by removing a random element.
                    0 => {
                        values.remove(test_runner.rng().random_range(0..values.len()));
                    }
                    // Increase array size.
                    1 => values.push(new_value(param_type, test_runner)),
                    // Mutate random array element.
                    2 => mutate_random_array_value(&mut values, param_type, test_runner, state),
                    _ => unreachable!(),
                }
                Some(DynSolValue::Array(values))
            } else {
                None
            }
        }
        DynSolValue::FixedArray(mut values) => {
            if let DynSolType::FixedArray(param_type, _size) = param
                && !values.is_empty()
            {
                mutate_random_array_value(&mut values, param_type, test_runner, state);
                Some(DynSolValue::FixedArray(values))
            } else {
                None
            }
        }
        DynSolValue::FixedBytes(word, size) => match test_runner.rng().random_range(0..=4) {
            0 => Word::flip_random_bit(word, size, test_runner),
            1 => Word::mutate_interesting_byte(word, size, test_runner),
            2 => Word::mutate_interesting_word(word, size, test_runner),
            3 => Word::mutate_interesting_dword(word, size, test_runner),
            4 => None,
            _ => unreachable!(),
        }
        .map(|word| DynSolValue::FixedBytes(word, size)),
        DynSolValue::CustomStruct { name, prop_names, tuple: mut values } => {
            if let DynSolType::CustomStruct { name: _, prop_names: _, tuple: tuple_types }
            | DynSolType::Tuple(tuple_types) = param
                && !values.is_empty()
            {
                // Mutate random struct element.
                mutate_random_tuple_value(&mut values, tuple_types, test_runner, state);
                Some(DynSolValue::CustomStruct { name, prop_names, tuple: values })
            } else {
                None
            }
        }
        DynSolValue::Tuple(mut values) => {
            if let DynSolType::Tuple(tuple_types) = param
                && !values.is_empty()
            {
                // Mutate random tuple element.
                mutate_random_tuple_value(&mut values, tuple_types, test_runner, state);
                Some(DynSolValue::Tuple(values))
            } else {
                None
            }
        }
        _ => None,
    }
    .unwrap_or_else(|| new_value(param, test_runner))
}

/// Mutates random value from given tuples.
fn mutate_random_tuple_value(
    tuple_values: &mut [DynSolValue],
    tuple_types: &[DynSolType],
    test_runner: &mut TestRunner,
    state: &EvmFuzzState,
) {
    let id = test_runner.rng().random_range(0..tuple_values.len());
    let param_type = &tuple_types[id];
    let old_val = replace(&mut tuple_values[id], DynSolValue::Bool(false));
    let new_val = mutate_param_value(param_type, old_val, test_runner, state);
    tuple_values[id] = new_val;
}

/// Mutates random value from given array.
fn mutate_random_array_value(
    array_values: &mut [DynSolValue],
    element_type: &DynSolType,
    test_runner: &mut TestRunner,
    state: &EvmFuzzState,
) {
    let elem = array_values.choose_mut(&mut test_runner.rng()).unwrap();
    let old_val = replace(elem, DynSolValue::Bool(false));
    let new_val = mutate_param_value(element_type, old_val, test_runner, state);
    *elem = new_val;
}

#[cfg(test)]
mod tests {
    use crate::{
        FuzzFixtures,
        strategies::{EvmFuzzState, fuzz_calldata, fuzz_calldata_from_state},
    };
    use foundry_common::abi::get_func;
    use foundry_config::FuzzDictionaryConfig;
    use revm::database::{CacheDB, EmptyDB};

    #[test]
    fn can_fuzz_array() {
        let f = "testArray(uint64[2] calldata values)";
        let func = get_func(f).unwrap();
        let db = CacheDB::new(EmptyDB::default());
        let state = EvmFuzzState::new(&db, FuzzDictionaryConfig::default(), &[]);
        let strategy = proptest::prop_oneof![
            60 => fuzz_calldata(func.clone(), &FuzzFixtures::default()),
            40 => fuzz_calldata_from_state(func, &state),
        ];
        let cfg = proptest::test_runner::Config { failure_persistence: None, ..Default::default() };
        let mut runner = proptest::test_runner::TestRunner::new(cfg);
        let _ = runner.run(&strategy, |_| Ok(()));
    }
}
