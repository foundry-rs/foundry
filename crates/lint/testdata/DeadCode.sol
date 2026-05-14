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

abstract contract ArrayOverrideBase {
    function fixedArrayHook(uint256[2] memory values) internal virtual returns (uint256) {
        return values[0];
    }
}

contract ArrayOverrideChild is ArrayOverrideBase {
    function callFixedArrayHook() external pure returns (uint256) {
        uint256[2] memory values;
        return fixedArrayHook(values);
    }

    function fixedArrayHook(uint256[2] memory values) internal pure override returns (uint256) {
        return values[1];
    }
}

contract StaticBase {
    function usedViaStaticBase() internal pure returns (uint256) {
        return 1;
    }
}

contract StaticChild is StaticBase {
    function callStaticBase() external pure returns (uint256) {
        return StaticBase.usedViaStaticBase();
    }
}

contract OverloadedDeadCode {
    function callAddress(address value) external pure returns (uint256) {
        return overloaded(value);
    }

    function callAddressCast() external pure returns (uint256) {
        return overloadedCast(address(0));
    }

    function overloaded(address) internal pure returns (uint256) {
        return 1;
    }

    function overloaded(uint256) internal pure returns (uint256) {
        return 2;
    }

    function overloadedCast(address) internal pure returns (uint256) {
        return 3;
    }

    function overloadedCast(uint256) internal pure returns (uint256) {
        return 4;
    }
}

contract StaticOverloadBase {
    function usedViaStaticOverload(address) internal pure returns (uint256) {
        return 1;
    }

    function usedViaStaticOverload(uint256) internal pure returns (uint256) {
        return 2;
    }
}

contract StaticOverloadChild is StaticOverloadBase {
    function callStaticOverload() external pure returns (uint256) {
        return StaticOverloadBase.usedViaStaticOverload(address(0));
    }
}

contract AmbiguousOverloadReachability {
    function callFromHelper() external pure returns (uint256) {
        return ambiguous(makeAddress());
    }

    function makeAddress() internal pure returns (address) {
        return address(0);
    }

    function ambiguous(address) internal pure returns (uint256) {
        return 1;
    }

    function ambiguous(uint256) internal pure returns (uint256) {
        return 2;
    }
}

contract ImplicitConversionReachability {
    function callWidening(uint8 value) external pure returns (uint256) {
        return widened(value);
    }

    function widened(uint256 value) internal pure returns (uint256) {
        return value;
    }
}

contract PayableConversionReachability {
    function callPlain(address value) external pure returns (uint256) {
        return takesPlain(payable(value));
    }

    function callPayable(address value) external pure returns (uint256) {
        return takesPayable(payable(value));
    }

    function takesPlain(address) internal pure returns (uint256) {
        return 1;
    }

    function takesPayable(address payable) internal pure returns (uint256) {
        return 2;
    }
}

contract NamedArgOverload {
    function callNamed(address who, uint256 amount) external pure returns (uint256) {
        return named({amount: amount, who: who});
    }

    function named(address who, uint256 amount) internal pure returns (uint256) {
        return uint160(who) + amount;
    }

    function named(uint256 who, address amount) internal pure returns (uint256) {
        return who + uint160(amount);
    }
}
