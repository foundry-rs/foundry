#![cfg_attr(not(feature = "std"), no_std, no_main)]

extern crate alloc;
extern crate fluentbase_sdk;

use fluentbase_sdk::{
    basic_entrypoint,
    derive::{router, Contract},
    SharedAPI, U256,
};

#[derive(Contract, Default)]
struct PowerCalculator<SDK> {
    sdk: SDK,
}

pub trait PowerAPI {
    /// Calculate base^exponent
    fn power(&self, base: U256, exponent: U256) -> U256;
}

#[router(mode = "solidity")]
impl<SDK: SharedAPI> PowerAPI for PowerCalculator<SDK> {
    fn power(&self, base: U256, exponent: U256) -> U256 {
        // Simple implementation - be careful with large exponents!
        if exponent == U256::from(0) {
            return U256::from(1);
        }

        let mut result = U256::from(1);
        let mut exp = exponent;
        let mut base_pow = base;

        // Binary exponentiation
        while exp > U256::from(0) {
            if exp & U256::from(1) == U256::from(1) {
                result = result * base_pow;
            }
            base_pow = base_pow * base_pow;
            exp = exp >> 1;
        }

        result
    }
}

impl<SDK: SharedAPI> PowerCalculator<SDK> {
    pub fn deploy(&self) {}
}

basic_entrypoint!(PowerCalculator);

#[cfg(test)]
mod tests {
    use super::*;
    use fluentbase_sdk_testing::HostTestingContext;

    #[test]
    fn test_power_2_to_3() {
        // Create power(2, 3) call
        let base = U256::from(2);
        let exponent = U256::from(3);
        let input = PowerCall::new((base, exponent)).encode();

        // Execute contract
        let sdk = HostTestingContext::default().with_input(input);
        let mut calculator = PowerCalculator::new(sdk.clone());
        calculator.main();

        // Check result
        let encoded_output = &sdk.take_output();
        let output = PowerReturn::decode(&encoded_output.as_slice()).unwrap();
        assert_eq!(output.0, U256::from(8)); // 2^3 = 8
    }
}
