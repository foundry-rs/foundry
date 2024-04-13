mod int;
pub use int::IntStrategy;

mod uint;
pub use uint::UintStrategy;

mod param;
pub use param::{fuzz_param, fuzz_param_from_state};

mod calldata;
pub use calldata::{fuzz_calldata, fuzz_calldata_from_state};

mod state;
pub use state::{build_initial_state, collect_created_contracts, EvmFuzzState};

mod invariants;
pub use invariants::{fuzz_contract_with_calldata, invariant_strat, override_call_strat};

/// Macro to create strategy with fixtures.
/// 1. A default strategy if no fixture defined for current parameter.
/// 2. A weighted strategy that use fixtures and default strategy values for current parameter.
/// If fixture is not of the same type as fuzzed parameter then fuzzer will panic.
macro_rules! fixture_strategy {
    ($fixtures:ident, $default_strategy:expr) => {
        if let Some(fixtures) = $fixtures {
            proptest::prop_oneof![
                50 => {
                    let custom_fixtures: Vec<DynSolValue> =
                        fixtures.iter().enumerate().map(|(_, value)| value.to_owned()).collect();
                    let custom_fixtures_len = custom_fixtures.len();
                    any::<prop::sample::Index>()
                        .prop_map(move |index| {
                            let index = index.index(custom_fixtures_len);
                            custom_fixtures.get(index).unwrap().clone()
                        })
                },
                50 => $default_strategy
            ].boxed()
        } else {
            $default_strategy.boxed()
        }
    };
}

pub(crate) use fixture_strategy;
