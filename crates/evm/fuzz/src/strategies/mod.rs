mod address;
pub use address::AddressStrategy;

mod bytes;
pub use bytes::{BytesStrategy, FixedBytesStrategy};

mod int;
pub use int::IntStrategy;

mod uint;
pub use uint::UintStrategy;

mod param;
pub use param::{fuzz_param, fuzz_param_from_state};

mod calldata;
pub use calldata::{fuzz_calldata, fuzz_calldata_from_state};

mod state;
pub use state::{
    build_initial_state, collect_created_contracts, collect_state_from_call, EvmFuzzState,
};

mod string;
pub use string::StringStrategy;

mod invariants;
pub use invariants::{fuzz_contract_with_calldata, invariant_strat, override_call_strat};

/// Macro to create strategy with fixtures.
/// 1. A default strategy if no fixture defined for current parameter.
/// 2. A fixture based strategy if configured values for current parameter.
/// If fixture is not a valid type then an error is raised and test suite will continue to execute
/// with random values.
macro_rules! fixture_strategy {
    ($fixtures:ident, $value_from_fixture:expr, $default_strategy:expr) => {
        if let Some(fixtures) = $fixtures {
            let custom_fixtures: Vec<DynSolValue> =
                fixtures.iter().enumerate().map(|(_, value)| value.to_owned()).collect();
            let custom_fixtures_len = custom_fixtures.len();
            any::<prop::sample::Index>()
                .prop_map(move |index| {
                    // Generate value tree from fixture.
                    // If fixture is not a valid type, raise error and generate random value.
                    let index = index.index(custom_fixtures_len);
                    $value_from_fixture(custom_fixtures.get(index))
                })
                .boxed()
        } else {
            return $default_strategy
        }
    };
}

pub(crate) use fixture_strategy;
