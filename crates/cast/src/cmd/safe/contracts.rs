// The Safe ABI fixes the number of transaction hash fields.
#![allow(clippy::too_many_arguments)]

use alloy_primitives::{Address, address};
use alloy_sol_types::sol;

pub(super) const SAFE_V1_4_1: Address = address!("41675C099F32341bf84BFc5382aF534df5C7461a");
pub(super) const SAFE_L2_V1_4_1: Address = address!("29fcB43b46531BcA003ddC8FCB67FFE91900C762");
pub(super) const SAFE_PROXY_FACTORY_V1_4_1: Address =
    address!("4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67");
pub(super) const COMPATIBILITY_FALLBACK_HANDLER_V1_4_1: Address =
    address!("fd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99");
pub(super) const SIMULATE_TX_ACCESSOR_V1_4_1: Address =
    address!("3d4BA2E0884aa488718476ca2FB8Efc291A46199");
pub(super) const SENTINEL_OWNER: Address = address!("0000000000000000000000000000000000000001");
pub(super) const PREDETERMINED_SALT_NONCE: &str =
    "0xb1073742015cbcf5a3a4d9d1ae33ecf619439710b89475f92e2abd2117e90f90";

sol! {
    #[sol(rpc)]
    interface ISafe {
        function nonce() external view returns (uint256);

        function setup(
            address[] calldata owners,
            uint256 threshold,
            address to,
            bytes calldata data,
            address fallbackHandler,
            address paymentToken,
            uint256 payment,
            address payable paymentReceiver
        ) external;

        function getTransactionHash(
            address to,
            uint256 value,
            bytes calldata data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address refundReceiver,
            uint256 nonce
        ) external view returns (bytes32);

        function simulateAndRevert(address targetContract, bytes calldata calldataPayload) external;

        function execTransaction(
            address to,
            uint256 value,
            bytes calldata data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address payable refundReceiver,
            bytes calldata signatures
        ) external payable returns (bool success);

        event ExecutionSuccess(bytes32 indexed txHash, uint256 payment);
        event ExecutionFailure(bytes32 indexed txHash, uint256 payment);
    }

    interface ISimulateTxAccessor {
        function simulate(
            address to,
            uint256 value,
            bytes calldata data,
            uint8 operation
        ) external returns (uint256 estimate, bool success, bytes memory returnData);
    }

    #[sol(rpc)]
    interface ISafeProxyFactory {
        event ProxyCreation(address indexed proxy, address singleton);

        function createProxyWithNonce(
            address singleton,
            bytes memory initializer,
            uint256 saltNonce
        ) public returns (address proxy);
    }
}
