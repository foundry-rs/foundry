// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface Vm {
    function deal(address, uint256) external;
}

contract NativeReentrantHandler {
    bool private active;
    bool public broken;
    address payable constant RECEIVER = payable(address(0x1234));

    function action() external {
        active = true;
        (bool ok,) = RECEIVER.call{value: 1}("");
        require(ok, "transfer failed");
        active = false;
    }

    function callback() external {
        if (active) {
            broken = true;
            (bool ok,) = RECEIVER.call("");
            require(ok, "nested call failed");
            (ok,) = RECEIVER.call("");
            require(ok, "second nested call failed");
        }
    }
}

contract NativeReentrantInvariantTest {
    NativeReentrantHandler handler;

    function setUp() public {
        handler = new NativeReentrantHandler();
        Vm(address(uint160(uint256(keccak256("hevm cheat code"))))).deal(address(handler), 1 ether);
    }

    function targetContracts() public view returns (address[] memory targets) {
        targets = new address[](1);
        targets[0] = address(handler);
    }

    function invariant_noReentrancy() public view {
        require(!handler.broken(), "reentrant callback");
    }
}
