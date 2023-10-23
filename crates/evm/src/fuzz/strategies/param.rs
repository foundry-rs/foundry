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
    // TODO: How to reintegrate while already using the proptest traits from [DynSolValue]?
    // let value = any::<prop::sample::Index>()
    //     .prop_map(move |index| index.index(state_len))
    //     .prop_map(move |index| *st.read().values().iter().nth(index).unwrap());

    // Convert the value based on the parameter type
    match param {
        DynSolType::Address |
        DynSolType::Bytes |
        DynSolType::Int(_) |
        DynSolType::Uint(_) |
        DynSolType::FixedBytes(_) |
        DynSolType::Array(_) |
        DynSolType::FixedArray(_, _) |
        DynSolType::Tuple(_) |
        DynSolType::Function |
        DynSolType::Bool => DynSolValue::type_strategy(param).boxed(),
        DynSolType::String => DynSolValue::type_strategy(param)
            .prop_map(move |value| {
                DynSolValue::String(
                    String::from_utf8_lossy(value.as_str().unwrap().as_bytes())
                        .trim()
                        .trim_end_matches('\0')
                        .to_string(),
                )
            })
            .boxed(),
        DynSolType::CustomStruct { name, prop_names, tuple } => panic!("unsupported type"),
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
