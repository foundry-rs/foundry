// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.20;

interface IActivationRegistry {
    function admin() external view returns (address);
}

interface IB20Factory {
    function isB20(address token) external view returns (bool);
}

interface Vm {
    function deployCode(string calldata artifactPath) external returns (address);
}

contract NestedBaseCanary {
    IActivationRegistry internal constant ACTIVATION_REGISTRY =
        IActivationRegistry(0x8453000000000000000000000000000000000001);

    address public activationAdmin;

    constructor() {
        activationAdmin = ACTIVATION_REGISTRY.admin();
    }
}

contract BaseEvmTest {
    Vm internal constant vm = Vm(address(bytes20(uint160(uint256(keccak256("hevm cheat code"))))));
    address internal constant ACTIVATION_REGISTRY = 0x8453000000000000000000000000000000000001;
    address internal constant B20_FACTORY = address(bytes20(hex"B20F000000000000000000000000000000000000"));
    address internal constant MAINNET_BERYL_ACTIVATION_ADMIN =
        address(bytes20(hex"ce3a3bee7e72e2a24079f3c0cb3b97740ed425a9"));

    function test_azul_excludes_beryl_precompiles() public view {
        (bool activationSuccess, bytes memory activationOutput) =
            ACTIVATION_REGISTRY.staticcall(abi.encodeWithSelector(IActivationRegistry.admin.selector));
        require(activationSuccess, "activation registry call failed");
        require(activationOutput.length == 0, "activation registry is installed");

        (bool factorySuccess, bytes memory factoryOutput) =
            B20_FACTORY.staticcall(abi.encodeCall(IB20Factory.isB20, (B20_FACTORY)));
        require(factorySuccess, "B20 factory call failed");
        require(factoryOutput.length == 0, "B20 factory is installed");
    }

    function test_beryl_installs_native_precompiles() public view {
        require(
            IActivationRegistry(ACTIVATION_REGISTRY).admin() == MAINNET_BERYL_ACTIVATION_ADMIN,
            "unexpected activation admin"
        );
        require(!IB20Factory(B20_FACTORY).isB20(B20_FACTORY), "factory identified as B20");
    }

    function test_beryl_nested_deploy_code_uses_base_evm() public {
        address deployed = vm.deployCode("test/BaseEvm.t.sol:NestedBaseCanary");
        require(
            NestedBaseCanary(deployed).activationAdmin() == MAINNET_BERYL_ACTIVATION_ADMIN,
            "nested EVM used the wrong activation admin"
        );
    }
}
