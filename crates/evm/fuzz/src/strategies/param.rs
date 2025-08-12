use super::state::EvmFuzzState;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{Address, B256, I256, Sign, U256};
use proptest::{prelude::*, test_runner::TestRunner};
use rand::{SeedableRng, rngs::StdRng, seq::IndexedRandom};

/// The max length of arrays we fuzz for is 256.
const MAX_ARRAY_LEN: usize = 256;

// Interesting 8-bit values to inject.
static INTERESTING_8: [i8; 9] = [-128, -1, 0, 1, 16, 32, 64, 100, 127];

/// Interesting 16-bit values to inject.
static INTERESTING_16: [i16; 19] = [
    -128, -1, 0, 1, 16, 32, 64, 100, 127, -32768, -129, 128, 255, 256, 512, 1000, 1024, 4096, 32767,
];

/// Interesting 32-bit values to inject.
static INTERESTING_32: [i32; 27] = [
    -128,
    -1,
    0,
    1,
    16,
    32,
    64,
    100,
    127,
    -32768,
    -129,
    128,
    255,
    256,
    512,
    1000,
    1024,
    4096,
    32767,
    -2147483648,
    -100663046,
    -32769,
    32768,
    65535,
    65536,
    100663045,
    2147483647,
];

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
        // flip boolean value
        DynSolValue::Bool(val) => {
            trace!(target: "abi_mutation", "Bool flip {val}");
            DynSolValue::Bool(!val)
        }
        // Uint: increment / decrement, flip random bit, mutate with interesting words or generate
        // new value from state.
        DynSolValue::Uint(val, size) => match test_runner.rng().random_range(0..=5) {
            0 => {
                let mutated_val = if test_runner.rng().random::<bool>() {
                    val.saturating_add(U256::ONE) }
                else {
                    val.saturating_sub(U256::ONE)
                };
                trace!(target: "abi_mutation", "U256 increment/decrement {val} -> {mutated_val}");
                DynSolValue::Uint(mutated_val, size)
            }
            1 => flip_random_uint_bit(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "U256 flip random bit: {val} -> {mutated_val}");
                    DynSolValue::Uint(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            2 => mutate_interesting_uint_byte(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "U256 interesting byte: {val} -> {mutated_val}");
                    DynSolValue::Uint(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            3 => mutate_interesting_uint_word(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "U256 interesting word: {val} -> {mutated_val}");
                    DynSolValue::Uint(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            4 => mutate_interesting_uint_dword(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "U256 interesting dword: {val} -> {mutated_val}");
                    DynSolValue::Uint(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            5 => new_value(param, test_runner),
            _ => unreachable!(),
        },
        // Int: increment / decrement, flip random bit, mutate with interesting words or generate
        // new value from state.
        DynSolValue::Int(val, size) => match test_runner.rng().random_range(0..=5) {
            0 => {
                let mutated_val = if test_runner.rng().random::<bool>() {
                    val.saturating_add(I256::ONE) }
                else {
                    val.saturating_sub(I256::ONE)
                };
                trace!(target: "abi_mutation", "I256 increment/decrement {val} -> {mutated_val}");
                DynSolValue::Int(mutated_val, size)
            },
            1 => flip_random_int_bit(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "I256 flip random bit: {val} -> {mutated_val}");
                    DynSolValue::Int(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            2 => mutate_interesting_int_byte(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "I256 interesting byte: {val} -> {mutated_val}");
                    DynSolValue::Int(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            3 => mutate_interesting_int_word(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "I256 interesting word: {val} -> {mutated_val}");
                    DynSolValue::Int(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            4 => mutate_interesting_int_dword(val, size, test_runner)
                .map(|mutated_val| {
                    trace!(target: "abi_mutation", "I256 interesting dword: {val} -> {mutated_val}");
                    DynSolValue::Int(mutated_val, size)
                })
                .unwrap_or_else(|| new_value(param, test_runner)),
            5 => new_value(param, test_runner),
            _ => unreachable!(),
        },
        // Address: flip random bit or generate new value from state.
        DynSolValue::Address(val) => match test_runner.rng().random_range(0..=1) {
            0 => DynSolValue::Address(flip_random_bit_address(val, test_runner)),
            1 => new_value(param, test_runner),
            _ => unreachable!(),
        },
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
                    2 => mutate_array(&mut values, param_type, test_runner, state),
                    _ => unreachable!(),
                }
                DynSolValue::Array(values)
            } else {
                new_value(param, test_runner)
            }
        }
        DynSolValue::FixedArray(mut values) => {
            if let DynSolType::FixedArray(param_type, _size) = param
                && !values.is_empty()
            {
                mutate_array(&mut values, param_type, test_runner, state);
                DynSolValue::FixedArray(values)
            } else {
                new_value(param, test_runner)
            }
        }
        DynSolValue::CustomStruct { name, prop_names, tuple: mut values } => {
            if let DynSolType::CustomStruct { name: _, prop_names: _, tuple: tuple_types }
            | DynSolType::Tuple(tuple_types) = param
                && !values.is_empty()
            {
                // Mutate random struct element.
                mutate_tuple(&mut values, tuple_types, test_runner, state);
                DynSolValue::CustomStruct { name, prop_names, tuple: values }
            } else {
                new_value(param, test_runner)
            }
        }
        DynSolValue::Tuple(mut values) => {
            if let DynSolType::Tuple(tuple_types) = param
                && !values.is_empty()
            {
                // Mutate random tuple element.
                mutate_tuple(&mut values, tuple_types, test_runner, state);
                DynSolValue::Tuple(values)
            } else {
                new_value(param, test_runner)
            }
        }
        _ => new_value(param, test_runner),
    }
}

/// Mutates random value from given tuples.
fn mutate_tuple(
    tuples: &mut [DynSolValue],
    tuple_types: &[DynSolType],
    test_runner: &mut TestRunner,
    state: &EvmFuzzState,
) {
    let id = test_runner.rng().random_range(0..tuples.len());
    let param_type = &tuple_types[id];
    let new_val = mutate_param_value(param_type, tuples[id].clone(), test_runner, state);
    tuples[id] = new_val;
}

/// Mutates random value from given array.
fn mutate_array(
    array_values: &mut [DynSolValue],
    array_type: &DynSolType,
    test_runner: &mut TestRunner,
    state: &EvmFuzzState,
) {
    let id = test_runner.rng().random_range(0..array_values.len());
    let new_val = mutate_param_value(array_type, array_values[id].clone(), test_runner, state);
    array_values[id] = new_val;
}

/// Flips a single random bit in the given U256 value.
fn flip_random_uint_bit(value: U256, size: usize, test_runner: &mut TestRunner) -> Option<U256> {
    let bit_index = test_runner.rng().random_range(0..size);
    let mask = U256::from(1u8) << bit_index;
    validate_uint_mutation(value, value ^ mask, size)
}

/// Mutate using interesting bytes, None if it doesn't fit in current size.
fn mutate_interesting_uint_byte(
    value: U256,
    size: usize,
    test_runner: &mut TestRunner,
) -> Option<U256> {
    let mut bytes: [u8; 32] = value.to_be_bytes();
    let byte_index = test_runner.rng().random_range(0..32);
    let interesting = *INTERESTING_8.choose(&mut test_runner.rng()).unwrap() as u8;
    bytes[byte_index] = interesting;
    validate_uint_mutation(value, U256::from_be_bytes(bytes), size)
}

// Function to mutate a U256 by replacing word with an interesting value.
fn mutate_interesting_uint_word(
    value: U256,
    size: usize,
    test_runner: &mut TestRunner,
) -> Option<U256> {
    let mut bytes: [u8; 32] = value.to_be_bytes();
    let word_index = test_runner.rng().random_range(0..16);
    let interesting = *INTERESTING_16.choose(&mut test_runner.rng()).unwrap() as u16;
    let start = word_index * 2;
    bytes[start..start + 2].copy_from_slice(&interesting.to_be_bytes());
    validate_uint_mutation(value, U256::from_be_bytes(bytes), size)
}

// Function to mutate a U256 by replacing dword with an interesting value.
fn mutate_interesting_uint_dword(
    value: U256,
    size: usize,
    test_runner: &mut TestRunner,
) -> Option<U256> {
    let mut bytes: [u8; 32] = value.to_be_bytes();
    let word_index = test_runner.rng().random_range(0..8);
    let interesting = *INTERESTING_32.choose(&mut test_runner.rng()).unwrap() as u32;
    // Replace the 4 bytes of the selected word
    let start = word_index * 4;
    bytes[start..start + 4].copy_from_slice(&interesting.to_be_bytes());
    validate_uint_mutation(value, U256::from_be_bytes(bytes), size)
}

/// Returns mutated uint value if different than the original value and if it fits in the given
/// size, otherwise None.
fn validate_uint_mutation(original_value: U256, mutated_value: U256, size: usize) -> Option<U256> {
    let max_value = if size < 256 { (U256::from(1) << size) - U256::from(1) } else { U256::MAX };
    if original_value != mutated_value && mutated_value < max_value {
        Some(mutated_value)
    } else {
        None
    }
}

/// Flips a single random bit in the given I256 value.
fn flip_random_int_bit(value: I256, size: usize, test_runner: &mut TestRunner) -> Option<I256> {
    let bit_index = test_runner.rng().random_range(0..size);
    let (sign, mut abs): (Sign, U256) = value.into_sign_and_abs();
    abs ^= U256::from(1u8) << bit_index;
    validate_int_mutation(value, I256::checked_from_sign_and_abs(sign, abs)?, size)
}

/// Mutate using interesting bytes, None if it doesn't fit in current size.
fn mutate_interesting_int_byte(
    value: I256,
    size: usize,
    test_runner: &mut TestRunner,
) -> Option<I256> {
    let mut bytes: [u8; 32] = value.to_be_bytes();
    let byte_index = test_runner.rng().random_range(0..32);
    let interesting = *INTERESTING_8.choose(&mut test_runner.rng()).unwrap() as u8;
    bytes[byte_index] = interesting;
    validate_int_mutation(value, I256::from_be_bytes(bytes), size)
}

// Function to mutate an I256 by replacing word with an interesting value.
fn mutate_interesting_int_word(
    value: I256,
    size: usize,
    test_runner: &mut TestRunner,
) -> Option<I256> {
    let mut bytes: [u8; 32] = value.to_be_bytes();
    let word_index = test_runner.rng().random_range(0..16);
    let interesting = *INTERESTING_16.choose(&mut test_runner.rng()).unwrap() as u16;
    let start = word_index * 2;
    bytes[start..start + 2].copy_from_slice(&interesting.to_be_bytes());
    validate_int_mutation(value, I256::from_be_bytes(bytes), size)
}

// Function to mutate an I256 by replacing dword with an interesting value.
fn mutate_interesting_int_dword(
    value: I256,
    size: usize,
    test_runner: &mut TestRunner,
) -> Option<I256> {
    let mut bytes: [u8; 32] = value.to_be_bytes();
    let word_index = test_runner.rng().random_range(0..8);
    let interesting = *INTERESTING_32.choose(&mut test_runner.rng()).unwrap() as u32;
    let start = word_index * 4;
    bytes[start..start + 4].copy_from_slice(&interesting.to_be_bytes());
    validate_int_mutation(value, I256::from_be_bytes(bytes), size)
}

/// Returns mutated int value if different than the original value and if it fits in the given size,
/// otherwise None.
fn validate_int_mutation(original_value: I256, mutated_value: I256, size: usize) -> Option<I256> {
    let umax: U256 = (U256::from(1) << (size - 1)) - U256::from(1);
    if original_value != mutated_value
        && match mutated_value.sign() {
            Sign::Positive => {
                mutated_value < I256::overflowing_from_sign_and_abs(Sign::Positive, umax).0
            }
            Sign::Negative => {
                mutated_value >= I256::overflowing_from_sign_and_abs(Sign::Negative, umax).0
            }
        }
    {
        Some(mutated_value)
    } else {
        None
    }
}

/// Flips a single random bit in the given Address.
fn flip_random_bit_address(addr: Address, test_runner: &mut TestRunner) -> Address {
    let bit_index = test_runner.rng().random_range(0..160);
    let mut bytes = addr.0;
    bytes[bit_index / 8] ^= 1 << (bit_index % 8);
    let mutated_val = Address::from(bytes);
    trace!(target: "abi_mutation", "Address flip random bit: {addr} -> {mutated_val}");
    mutated_val
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
