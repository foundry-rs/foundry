// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface VmCaseIsolation {
    function assume(bool condition) external;
    function warp(uint256 timestamp) external;
    function roll(uint256 height) external;
    function startPrank(address sender) external;
    function store(address target, bytes32 slot, bytes32 value) external;
    function etch(address target, bytes calldata code) external;
}

abstract contract FuzzCaseIsolation {
    VmCaseIsolation constant vm = VmCaseIsolation(address(uint160(uint256(keccak256("hevm cheat code")))));
    address constant TARGET = address(0x10000);
    address constant PRANK = address(0x20000);
    uint256 counter;

    function setUp() public virtual {
        counter = 7;
        vm.warp(123);
        vm.roll(456);
    }

    function sender() external view returns (address) {
        return msg.sender;
    }

    function testFuzzAcceptedCaseIsolation(uint256) public {
        checkAndMutate();
    }

    function testFuzzRejectedCaseIsolation(bool accept) public {
        checkAndMutate();
        // Rejection must discard both EVM writes and host-side environment/prank changes.
        vm.assume(accept);
    }

    function checkAndMutate() internal {
        require(counter == 7, "case storage leaked");
        require(block.timestamp == 123, "case timestamp leaked");
        require(block.number == 456, "case block leaked");
        require(this.sender() == address(this), "case prank leaked");
        (bool ok, bytes memory output) = TARGET.staticcall("");
        require(ok && abi.decode(output, (uint256)) == 41, "backing storage leaked");

        counter = 8;
        vm.store(TARGET, bytes32(0), bytes32(uint256(99)));
        (ok, output) = TARGET.staticcall("");
        require(ok && abi.decode(output, (uint256)) == 99, "case write missing");
        vm.warp(124);
        vm.roll(457);
        vm.startPrank(PRANK);
        require(this.sender() == PRANK, "case prank missing");
        // Intentionally leave the prank and environment mutations active at the case boundary.
    }
}

contract LocalFuzzCaseIsolationTest is FuzzCaseIsolation {
    function setUp() public override {
        super.setUp();
        // Return storage slot zero for any calldata.
        vm.etch(TARGET, hex"60005460005260206000f3");
        vm.store(TARGET, bytes32(0), bytes32(uint256(41)));
    }
}

// The CLI test supplies TARGET's code and storage through the fork's backing RPC.
contract ForkFuzzCaseIsolationTest is FuzzCaseIsolation {}
