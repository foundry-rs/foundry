//! ABIs of Arc's precompiles, so their calls and events decode by name.

alloy_sol_types::sol! {
    #[sol(abi)]
    interface INativeCoinAuthority {
        function mint(address to, uint256 amount) external returns (bool);
        function burn(address from, uint256 amount) external returns (bool);
        function transfer(address from, address to, uint256 amount) external returns (bool);
        function totalSupply() external view returns (uint256 supply);
    }

    #[sol(abi)]
    interface INativeCoinControl {
        function blocklist(address account) external returns (bool success);
        function isBlocklisted(address account) external view returns (bool _isBlocklisted);
        function unBlocklist(address account) external returns (bool success);
        event Blocklisted(address indexed account);
        event UnBlocklisted(address indexed account);
    }

    #[sol(abi)]
    interface ISystemAccounting {
        struct GasValues {
            uint64 gasUsed;
            uint64 gasUsedSmoothed;
            uint64 nextBaseFee;
        }
        function storeGasValues(uint64 blockNumber, GasValues calldata gasValues) external returns (bool);
        function getGasValues(uint64 blockNumber) external view returns (GasValues calldata gasValue);
    }

    #[sol(abi)]
    interface ICallFrom {
        function callFrom(address sender, address target, bytes calldata data) external returns (bool success, bytes memory returnData);
    }

    #[sol(abi)]
    interface IPQ {
        function verifySlhDsaSha2128s(bytes calldata vk, bytes calldata message, bytes calldata sig) external returns (bool isValid);
    }
}
