use alloy_primitives::{address, hex, Address};

/// The cheatcode handler address.
///
/// This is the same address as the one used in DappTools's HEVM.
/// It is calculated as:
/// `address(bytes20(uint160(uint256(keccak256('hevm cheat code')))))`
pub const CHEATCODE_ADDRESS: Address = address!("7109709ECfa91a80626fF3989D68f67F5b1DD12D");

/// The Hardhat console address.
///
/// See: <https://github.com/nomiclabs/hardhat/blob/master/packages/hardhat-core/console.sol>
pub const HARDHAT_CONSOLE_ADDRESS: Address = address!("000000000000000000636F6e736F6c652e6c6f67");

/// Stores the caller address to be used as *sender* account for:
/// - deploying Test contracts
/// - deploying Script contracts
///
/// Derived from `address(uint160(uint256(keccak256("foundry default caller"))))`,
/// which is equal to `0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38`.
pub const CALLER: Address = address!("1804c8AB1F12E6bbf3894d4083f33e07309d1f38");

/// The default test contract address.
pub const TEST_CONTRACT_ADDRESS: Address = address!("b4c79daB8f259C7Aee6E5b2Aa729821864227e84");

/// Magic return value returned by the `assume` cheatcode.
pub const MAGIC_ASSUME: &[u8] = b"FOUNDRY::ASSUME";

/// Magic return value returned by the `skip` cheatcode.
pub const MAGIC_SKIP: &[u8] = b"FOUNDRY::SKIP";

/// The default CREATE2 deployer.
pub const DEFAULT_CREATE2_DEPLOYER: Address = address!("4e59b44847b379578588920ca78fbf26c0b4956c");
/// The initcode of the default CREATE2 deployer.
pub const DEFAULT_CREATE2_DEPLOYER_CODE: &[u8] = &hex!("604580600e600039806000f350fe7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3");
/// The runtime code of the default CREATE2 deployer.
pub const DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE: &[u8] = &hex!("7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3");
