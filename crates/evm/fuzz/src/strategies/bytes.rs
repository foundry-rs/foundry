use crate::strategies::fixture_strategy;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::B256;
use proptest::{
    arbitrary::any,
    prelude::{prop, BoxedStrategy},
    strategy::Strategy,
};

/// The bytes strategy combines 2 different strategies:
/// 1. A random bytes strategy if no fixture defined for current parameter.
/// 2. A fixture based strategy if configured values for current parameters.
/// If fixture is not a valid type then an error is raised and test suite will continue to execute
/// with random values.
///
///
/// For example:
/// To define fixture for `backup` fuzzed parameter, return an array of possible values from
/// `function fixture_backup() external pure returns (bytes[] memory)`.
/// Use `backup` named parameter in fuzzed test in order to create a custom strategy
/// `function testFuzz_backupValue(bytes memory backup)`.
#[derive(Debug, Default)]
pub struct BytesStrategy {}

impl BytesStrategy {
    /// Create a new bytes strategy.
    pub fn init(fixtures: Option<&[DynSolValue]>) -> BoxedStrategy<DynSolValue> {
        let value_from_fixture = |fixture: Option<&DynSolValue>| {
            if let Some(val @ DynSolValue::Bytes(_)) = fixture {
                return val.clone()
            }
            error!("{:?} is not a valid bytes fixture, generate random value", fixture);
            let random: [u8; 32] = rand::random();
            DynSolValue::Bytes(random.to_vec())
        };
        fixture_strategy!(
            fixtures,
            value_from_fixture,
            DynSolValue::type_strategy(&DynSolType::Bytes).boxed()
        )
    }
}

/// The fixed bytes strategy combines 2 different strategies:
/// 1. A random fixed bytes strategy if no fixture defined for current parameter.
/// 2. A fixture based strategy if configured values for current parameters.
/// If fixture is not a valid type then an error is raised and test suite will continue to execute
/// with random values.
///
///
/// For example:
/// To define fixture for `key` fuzzed parameter, return an array of possible values from
/// `function fixture_key() external pure returns (bytes32[] memory)`.
/// Use `key` named parameter in fuzzed test in order to create a custom strategy
/// `function testFuzz_keyValue(bytes32 key)`.
#[derive(Debug, Default)]
pub struct FixedBytesStrategy {}

impl FixedBytesStrategy {
    /// Create a new fixed bytes strategy.
    pub fn init(size: usize, fixtures: Option<&[DynSolValue]>) -> BoxedStrategy<DynSolValue> {
        let value_from_fixture = move |fixture: Option<&DynSolValue>| {
            if let Some(fixture) = fixture {
                if let Some(fixture) = fixture.as_fixed_bytes() {
                    if fixture.1 == size {
                        return DynSolValue::FixedBytes(B256::from_slice(fixture.0), fixture.1);
                    }
                }
            }
            error!("{:?} is not a valid fixed bytes fixture, generate random value", fixture);
            DynSolValue::FixedBytes(B256::random(), size)
        };
        fixture_strategy!(
            fixtures,
            value_from_fixture,
            any::<B256>()
                .prop_map(move |mut v| {
                    v[size..].fill(0);
                    DynSolValue::FixedBytes(v, size)
                })
                .boxed()
        )
    }
}
