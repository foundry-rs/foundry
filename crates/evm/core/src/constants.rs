use alloy_primitives::{Address, B256, address, b256, hex};

/// The cheatcode handler address.
///
/// This is the same address as the one used in DappTools's HEVM.
///
/// This is calculated as:
/// `address(bytes20(uint160(uint256(keccak256('hevm cheat code')))))`
pub const CHEATCODE_ADDRESS: Address = address!("0x7109709ECfa91a80626fF3989D68f67F5b1DD12D");

/// The contract hash at [`CHEATCODE_ADDRESS`].
///
/// This is calculated as:
/// `keccak256(abi.encodePacked(CHEATCODE_ADDRESS))`.
pub const CHEATCODE_CONTRACT_HASH: B256 =
    b256!("0xb0450508e5a2349057c3b4c9c84524d62be4bb17e565dbe2df34725a26872291");

/// The Hardhat console address.
///
/// See: <https://github.com/NomicFoundation/hardhat/blob/main/v-next/hardhat/console.sol>
pub const HARDHAT_CONSOLE_ADDRESS: Address = address!("0x000000000000000000636F6e736F6c652e6c6f67");

/// Stores the caller address to be used as *sender* account for:
/// - deploying Test contracts
/// - deploying Script contracts
///
/// Derived from `address(uint160(uint256(keccak256("foundry default caller"))))`,
/// which is equal to `0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38`.
pub const CALLER: Address = address!("0x1804c8AB1F12E6bbf3894d4083f33e07309d1f38");

/// The default test contract address.
///
/// Derived from `CALLER.create(1)`.
pub const TEST_CONTRACT_ADDRESS: Address = address!("0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496");

/// Magic return value returned by the `assume` cheatcode.
pub const MAGIC_ASSUME: &[u8] = b"FOUNDRY::ASSUME";

/// Magic return value returned by the `skip` cheatcode. Optionally appended with a reason.
pub const MAGIC_SKIP: &[u8] = b"FOUNDRY::SKIP";

/// The address that deploys the default CREATE2 deployer contract.
pub const DEFAULT_CREATE2_DEPLOYER_DEPLOYER: Address =
    address!("0x3fAB184622Dc19b6109349B94811493BF2a45362");
/// The default CREATE2 deployer.
pub const DEFAULT_CREATE2_DEPLOYER: Address =
    address!("0x4e59b44847b379578588920ca78fbf26c0b4956c");
/// The initcode of the default CREATE2 deployer.
pub const DEFAULT_CREATE2_DEPLOYER_CODE: &[u8] = &hex!(
    "604580600e600039806000f350fe7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3"
);
/// The runtime code of the default CREATE2 deployer.
pub const DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE: &[u8] = &hex!(
    "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffe03601600081602082378035828234f58015156039578182fd5b8082525050506014600cf3"
);
/// The hash of the default CREATE2 deployer code.
///
/// This is calculated as `keccak256([`DEFAULT_CREATE2_DEPLOYER_RUNTIME_CODE`])`.
pub const DEFAULT_CREATE2_DEPLOYER_CODEHASH: B256 =
    b256!("0x2fa86add0aed31f33a762c9d88e807c475bd51d0f52bd0955754b2608f7e4989");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create2_deployer() {
        assert_eq!(DEFAULT_CREATE2_DEPLOYER_DEPLOYER.create(0), DEFAULT_CREATE2_DEPLOYER);
    }
}
