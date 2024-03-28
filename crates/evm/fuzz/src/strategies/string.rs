use crate::strategies::fixture_strategy;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use proptest::{
    arbitrary::any,
    prelude::{prop, BoxedStrategy},
    strategy::Strategy,
};
use rand::{distributions::Alphanumeric, thread_rng, Rng};

/// The address strategy combines 2 different strategies:
/// 1. A random addresses strategy if no fixture defined for current parameter.
/// 2. A fixture based strategy if configured values for current parameters.
/// If fixture is not a valid type then an error is raised and test suite will continue to execute
// with random strings.
///
///
/// For example:
/// To define fixture for `person` fuzzed parameter, return an array of possible values from
/// `function fixture_person() public returns (string[] memory)`.
/// Use `person` named parameter in fuzzed test in order to create a custom strategy
/// `function testFuzz_personValue(string memory person)`.
#[derive(Debug, Default)]
pub struct StringStrategy {}

impl StringStrategy {
    /// Create a new string strategy.
    pub fn init(fixtures: Option<&[DynSolValue]>) -> BoxedStrategy<DynSolValue> {
        let value_from_fixture = |fixture: Option<&DynSolValue>| {
            if let Some(fixture) = fixture {
                if let Some(fixture) = fixture.as_str() {
                    return DynSolValue::String(fixture.to_string());
                }
            }
            error!("{:?} is not a valid string fixture, generate random value", fixture);
            let mut rng = thread_rng();
            let string_len = rng.gen_range(0..128);
            let random: String =
                (&mut rng).sample_iter(Alphanumeric).map(char::from).take(string_len).collect();
            DynSolValue::String(random)
        };
        fixture_strategy!(
            fixtures,
            value_from_fixture,
            DynSolValue::type_strategy(&DynSolType::String)
                .prop_map(move |value| {
                    DynSolValue::String(
                        value.as_str().unwrap().trim().trim_end_matches('\0').to_string(),
                    )
                })
                .boxed()
        )
    }
}
