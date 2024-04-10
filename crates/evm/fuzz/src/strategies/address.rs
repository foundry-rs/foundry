use crate::strategies::fixture_strategy;
use alloy_dyn_abi::DynSolValue;
use alloy_primitives::Address;
use proptest::{
    arbitrary::any,
    prelude::{prop, BoxedStrategy},
    strategy::Strategy,
};

/// The address strategy combines 2 different strategies:
/// 1. A random addresses strategy if no fixture defined for current parameter.
/// 2. A fixture based strategy if configured values for current parameters.
/// If fixture is not a valid type then an error is raised and test suite will continue to execute
/// with random address.
///
///
/// For example:
/// To define fixture for `owner` fuzzed parameter, return an array of possible values from
/// `function fixture_owner() public returns (address[] memory)`.
/// Use `owner` named parameter in fuzzed test in order to create a custom strategy
/// `function testFuzz_ownerAddress(address owner, uint amount)`.
#[derive(Debug, Default)]
pub struct AddressStrategy {}

impl AddressStrategy {
    /// Create a new address strategy.
    pub fn init(fixtures: Option<&[DynSolValue]>) -> BoxedStrategy<DynSolValue> {
        let value_from_fixture = |fixture: Option<&DynSolValue>| {
            if let Some(val @ DynSolValue::Address(_)) = fixture {
                return val.clone()
            }
            error!("{:?} is not a valid address fixture, generate random value", fixture);
            DynSolValue::Address(Address::random())
        };
        fixture_strategy!(
            fixtures,
            value_from_fixture,
            any::<Address>().prop_map(DynSolValue::Address).boxed()
        )
    }
}
