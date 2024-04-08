// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

interface Vm {
    function etch(address target, bytes calldata newRuntimeBytecode) external;
}

// https://github.com/foundry-rs/foundry/issues/5625
// https://github.com/foundry-rs/foundry/issues/6166
// `Target.wrongSelector` is not called when handler added as `targetContract`
// `Target.wrongSelector` is called (and test fails) when no `targetContract` set
contract Target {
    uint256 count;

    function wrongSelector() external {
        revert("wrong target selector called");
    }

    function goodSelector() external {
        count++;
    }
}

contract Handler is DSTest {
    function increment() public {
        Target(0x6B175474E89094C44Da98b954EedeAC495271d0F).goodSelector();
    }
}

contract ExplicitTargetContract is DSTest {
    Vm vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    Handler handler;

    function setUp() public {
        Target target = new Target();
        bytes memory targetCode = address(target).code;
        vm.etch(address(0x6B175474E89094C44Da98b954EedeAC495271d0F), targetCode);

        handler = new Handler();
    }

    function targetContracts() public returns (address[] memory) {
        address[] memory addrs = new address[](1);
        addrs[0] = address(handler);
        return addrs;
    }

    function invariant_explicit_target() public {}
}

contract DynamicTargetContract is DSTest {
    Vm vm = Vm(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D);
    Handler handler;

    function setUp() public {
        Target target = new Target();
        bytes memory targetCode = address(target).code;
        vm.etch(address(0x6B175474E89094C44Da98b954EedeAC495271d0F), targetCode);

        handler = new Handler();
    }

    function invariant_dynamic_targets() public {}
}
