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
/// To define fixtures for `owner` fuzzed parameter, return an array of possible values from
/// `function fixtures_owner() public returns (address[] memory)`.
/// Use `owner` named parameter in fuzzed test in order to create a custom strategy
/// `function testFuzz_ownerAddress(address owner, uint amount)`.
#[derive(Debug)]
pub struct AddressStrategy {}

impl AddressStrategy {
    /// Create a new address strategy.
    pub fn init(fixtures: Option<&[DynSolValue]>) -> BoxedStrategy<DynSolValue> {
        if let Some(fixtures) = fixtures {
            let address_fixtures: Vec<DynSolValue> =
                fixtures.iter().enumerate().map(|(_, value)| value.to_owned()).collect();
            let address_fixtures_len = address_fixtures.len();
            any::<prop::sample::Index>()
                .prop_map(move |index| {
                    // Generate value tree from fixture.
                    // If fixture is not a valid address, raise error and generate random value.
                    let index = index.index(address_fixtures_len);
                    if let Some(addr_fixture) = address_fixtures.get(index) {
                        if let Some(addr_fixture) = addr_fixture.as_address() {
                            return DynSolValue::Address(addr_fixture);
                        }
                    }
                    error!(
                        "{:?} is not a valid address fixture, generate random value",
                        address_fixtures.get(index)
                    );
                    DynSolValue::Address(Address::random())
                })
                .boxed()
        } else {
            // If no config for addresses dictionary then create unbounded addresses strategy.
            any::<Address>().prop_map(DynSolValue::Address).boxed()
        }
    }
}
