//@compile-flags: --only-lint calls-loop reentrancy-events

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

struct LedgerRow {
    address wallet;
    uint256 amount;
}

interface LengthSource {
    function nextLength() external returns (uint256);
    function length() external view returns (uint256);
}

contract Child {}

contract CallsLoopAllocations {
    event Allocated();
    address[] wallets;

    // Memory allocation does not interact with another contract, even for contract arrays.
    function allocateInLoop(uint256 n) external returns (LedgerRow[] memory rows) {
        for (uint256 i = 0; i < n; ++i) {
            rows = new LedgerRow[](wallets.length);
            uint256[] memory numbers = new uint256[](n);
            bytes memory data = new bytes(n);
            string memory text = new string(n);
            Child[] memory children = new Child[](n);
            LedgerRow[][] memory nested = new LedgerRow[][](n);
            emit Allocated();
        }
    }

    // The loop walker also follows allocations inside local helpers.
    function allocateThroughHelper(uint256 n) external returns (LedgerRow[] memory rows) {
        for (uint256 i = 0; i < n; ++i) {
            rows = allocate(n);
            emit Allocated();
        }
    }

    function allocate(uint256 n) internal pure returns (LedgerRow[] memory) {
        return new LedgerRow[](n);
    }

    // The shared reentrancy classifier must not treat a standalone allocation as an interaction.
    function allocateThenEmit(uint256 n) external returns (bytes memory data) {
        data = new bytes(n);
        emit Allocated();
    }

    // Only the length calculation is an external call, not the enclosing allocation.
    function externalLength(LengthSource source) external {
        for (uint256 i = 0; i < 2; ++i) {
            uint256[] memory numbers = new uint256[](source.nextLength()); //~WARN: external call inside a loop
            emit Allocated(); //~WARN: event emitted after an external call
        }
    }

    // A view call is still external, but cannot affect log ordering.
    function viewLength(LengthSource source) external {
        for (uint256 i = 0; i < 2; ++i) {
            uint256[] memory numbers = new uint256[](source.length()); //~WARN: external call inside a loop
            emit Allocated();
        }
    }

    // An allocation must not erase an earlier real interaction.
    function callThenAllocate(LengthSource source) external {
        uint256 n = source.nextLength();
        bytes memory data = new bytes(n);
        emit Allocated(); //~WARN: event emitted after an external call
    }

    // Actual contract creation remains an external interaction, with or without a salt.
    function createContracts() external {
        for (uint256 i = 0; i < 2; ++i) {
            new Child(); //~WARN: external call inside a loop
            emit Allocated(); //~WARN: event emitted after an external call
        }
        for (uint256 i = 0; i < 2; ++i) {
            new Child{salt: bytes32(i)}(); //~WARN: external call inside a loop
            emit Allocated(); //~WARN: event emitted after an external call
        }
    }
}
