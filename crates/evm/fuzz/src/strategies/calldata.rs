use crate::strategies::{fuzz_param, EvmFuzzState};
use alloy_dyn_abi::JsonAbiExt;
use alloy_json_abi::Function;
use alloy_primitives::{Address, Bytes};
use foundry_config::FuzzDictionaryConfig;
use hashbrown::HashSet;
use proptest::prelude::{BoxedStrategy, Strategy};
use std::{fmt, sync::Arc};

pub type CalldataFuzzDictionary = Arc<CalldataFuzzDictionaryConfig>;

#[derive(Clone)]
pub struct CalldataFuzzDictionaryConfig {
    /// Addresses that can be used for fuzzing calldata.
    pub addresses: Vec<Address>,
}

impl fmt::Debug for CalldataFuzzDictionaryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalldataFuzzDictionaryConfig").field("addresses", &self.addresses).finish()
    }
}

impl CalldataFuzzDictionaryConfig {
    pub fn new(config: &FuzzDictionaryConfig, state: EvmFuzzState) -> Self {
        let mut addresses: HashSet<Address> = HashSet::new();

        if let Some(len) = config.max_calldata_fuzz_dictionary_addresses {
            loop {
                if addresses.len() == len {
                    break
                }
                addresses.insert(Address::random());
            }

            // add any state address calldata fuzz dictionary, in addition to random generated
            // addresses
            let mut state = state.write();
            addresses.extend(state.addresses());
        }

        Self { addresses: Vec::from_iter(addresses) }
    }
}

/// Given a function, it returns a strategy which generates valid calldata
/// for that function's input types.
pub fn fuzz_calldata(func: Function) -> BoxedStrategy<Bytes> {
    fuzz_calldata_with_config(func, None)
}

pub fn fuzz_calldata_with_config(
    func: Function,
    config: Option<CalldataFuzzDictionary>,
) -> BoxedStrategy<Bytes> {
    // We need to compose all the strategies generated for each parameter in all
    // possible combinations
    let strats = func
        .inputs
        .iter()
        .map(|input| fuzz_param(&input.selector_type().parse().unwrap(), config.clone()))
        .collect::<Vec<_>>();

    strats
        .prop_map(move |tokens| {
            trace!(input=?tokens);
            func.abi_encode_input(&tokens).unwrap().into()
        })
        .boxed()
}
