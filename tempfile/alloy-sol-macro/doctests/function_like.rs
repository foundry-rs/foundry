use alloy_primitives::{hex, keccak256, U256};
use alloy_sol_types::{sol, SolCall, SolError};

sol! {
    function foo(uint256 a, uint256 b) external view returns (uint256);

    // These will generate structs prefixed with `overloaded_0`, `overloaded_1`,
    // and `overloaded_2` by default, but each signature is calculated with
    // `overloaded` as the function name.
    function overloaded();
    function overloaded(uint256) returns (uint256);
    function overloaded(string);

    // State variables will generate getter functions just like in Solidity.
    mapping(uint k => bool v) public variableGetter;

    /// Implements [`SolError`].
    #[derive(Debug, PartialEq, Eq)]
    error MyError(uint256 a, uint256 b);
}

#[test]
fn function() {
    assert_call_signature::<fooCall>("foo(uint256,uint256)");

    let call = fooCall { a: U256::from(1), b: U256::from(2) };
    let _call_data = call.abi_encode();

    let _ = overloaded_0Call {};
    assert_call_signature::<overloaded_0Call>("overloaded()");

    let _ = overloaded_1Call { _0: U256::from(1) };
    assert_call_signature::<overloaded_1Call>("overloaded(uint256)");

    let _ = overloaded_2Call { _0: "hello".into() };
    assert_call_signature::<overloaded_2Call>("overloaded(string)");

    // Exactly the same as `function variableGetter(uint256) returns (bool)`.
    let _ = variableGetterCall { k: U256::from(2) };
    assert_call_signature::<variableGetterCall>("variableGetter(uint256)");
    let _ = variableGetterReturn { v: false };
}

#[test]
fn error() {
    assert_error_signature::<MyError>("MyError(uint256,uint256)");
    let call_data = hex!(
        "0000000000000000000000000000000000000000000000000000000000000001"
        "0000000000000000000000000000000000000000000000000000000000000002"
    );
    assert_eq!(
        MyError::abi_decode_raw(&call_data, true),
        Ok(MyError { a: U256::from(1), b: U256::from(2) })
    );
}

fn assert_call_signature<T: SolCall>(expected: &str) {
    assert_eq!(T::SIGNATURE, expected);
    assert_eq!(T::SELECTOR, keccak256(expected)[..4]);
}

fn assert_error_signature<T: SolError>(expected: &str) {
    assert_eq!(T::SIGNATURE, expected);
    assert_eq!(T::SELECTOR, keccak256(expected)[..4]);
}
