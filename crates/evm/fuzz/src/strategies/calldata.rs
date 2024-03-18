use crate::strategies::{fuzz_param, EvmFuzzState};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes};
use foundry_config::FuzzDictionaryConfig;
use hashbrown::HashSet;
use proptest::prelude::Strategy;
use std::sync::Arc;

/// Clonable wrapper around [CalldataFuzzDictionary].
#[derive(Clone, Debug)]
pub struct CalldataFuzzDictionary {
    pub inner: Arc<CalldataFuzzDictionaryConfig>,
}

impl CalldataFuzzDictionary {
    pub fn new(config: &FuzzDictionaryConfig, state: &EvmFuzzState) -> Self {
        Self { inner: Arc::new(CalldataFuzzDictionaryConfig::new(config, state)) }
    }
}

#[derive(Clone, Debug)]
pub struct CalldataFuzzDictionaryConfig {
    /// Addresses that can be used for fuzzing calldata.
    pub addresses: Vec<Address>,
}

/// Represents custom configuration for invariant fuzzed calldata strategies.
///
/// At the moment only the dictionary of addresses to be used for a fuzzed `function(address)` can
/// be configured, but support for other types can be added.
impl CalldataFuzzDictionaryConfig {
    /// Creates config with the set of addresses that can be used for fuzzing invariant calldata (if
    /// `max_calldata_fuzz_dictionary_addresses` configured).
    /// The set of addresses contains a number of `max_calldata_fuzz_dictionary_addresses` random
    /// addresses plus all addresses that already had their PUSH bytes collected (retrieved from
    /// `EvmFuzzState`, if `include_push_bytes` config enabled).
    pub fn new(config: &FuzzDictionaryConfig, state: &EvmFuzzState) -> Self {
        let mut addresses = HashSet::<Address>::new();

        let dict_size = config.max_calldata_fuzz_dictionary_addresses;
        if dict_size > 0 {
            addresses.extend(std::iter::repeat_with(Address::random).take(dict_size));
            // Add all addresses that already had their PUSH bytes collected.
            addresses.extend(state.read().addresses());
        }

        Self { addresses: addresses.into_iter().collect() }
    }
}

/// Given a function, it returns a strategy which generates valid calldata
/// for that function's input types.
pub fn fuzz_calldata(func: Function) -> impl Strategy<Value = Bytes> {
    fuzz_calldata_with_config(func, None)
}

/// Given a function, it returns a strategy which generates valid calldata
/// for that function's input types, following custom configuration rules.
pub fn fuzz_calldata_with_config(
    func: Function,
    config: Option<&CalldataFuzzDictionary>,
) -> impl Strategy<Value = Bytes> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    let strats = func
        .inputs
        .iter()
        .map(|input| fuzz_param(&input.selector_type().parse().unwrap(), config))
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
