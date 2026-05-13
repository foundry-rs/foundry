//@compile-flags: --only-lint dead-code

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.18;

function unusedFreeFunction() pure returns (uint256) {
    return 1;
}

function usedFreeFunction() pure returns (uint256) {
    return 2;
}

contract DeadCode {
    uint256 initialized = usedFromInitializer();

    constructor() {
        usedFromConstructor();
    }

    receive() external payable {
        usedFromReceive();
    }

    fallback() external payable {
        usedFromFallback();
    }

    modifier usesHelper() {
        usedFromModifier();
        _;
    }

    function entry(uint256 value) external usesHelper returns (uint256) {
        return usedDirectly(value) + usedTransitively(value) + usedFreeFunction();
    }

    function publicEntry() public pure returns (uint256) {
        return 10;
    }

    function usedDirectly(uint256 value) internal pure returns (uint256) {
        return value;
    }

    function usedTransitively(uint256 value) private pure returns (uint256) {
        return usedLeaf(value);
    }

    function usedLeaf(uint256 value) internal pure returns (uint256) {
        return value + 1;
    }

    function usedFromConstructor() private {}

    function usedFromReceive() private {}

    function usedFromFallback() internal {}

    function usedFromInitializer() internal pure returns (uint256) {
        return 0;
    }

    function usedFromModifier() private {}

    function unusedInternal() internal pure returns (uint256) {
        return 3;
    }

    function unusedPrivate() private pure returns (uint256) {
        return 4;
    }

    function unreachableInternal() internal pure returns (uint256) {
        return unreachablePrivate();
    }

    function unreachablePrivate() private pure returns (uint256) {
        return 5;
    }
}

abstract contract AbstractDeclarations {
    function unimplemented() internal virtual returns (uint256);
}

abstract contract AbstractBase {
    function hook() internal virtual returns (uint256) {
        return 1;
    }
}

contract Child is AbstractBase {
    function callHook() external returns (uint256) {
        return hook();
    }

    function hook() internal pure override returns (uint256) {
        return 2;
    }
}
