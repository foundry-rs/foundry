//@compile-flags: --only-lint missing-inheritance

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {IExternalThing} from "./auxiliary/MissingInheritanceExternal.sol";

interface ISomething {
    function f1() external returns (uint256);
}

interface IExtra {
    function g() external view returns (uint256);
}

interface IERC20Like {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address who) external view returns (uint256);
}

interface IERC20LikeMetadata {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address who) external view returns (uint256);
    function name() external view returns (string memory);
}

// Signature-only abstract contract; treated as a candidate interface.
abstract contract IFooLike {
    function foo() external virtual returns (uint256);
}

// SHOULD FAIL: Implements ISomething but does not explicitly inherit from it.
contract Something { //~NOTE: contract `Something` implements interface `ISomething`'s external API but does not explicitly inherit from it
    function f1() external pure returns (uint256) {
        return 42;
    }
}

// SHOULD PASS: Explicitly inherits from ISomething.
contract SomethingExplicit is ISomething {
    function f1() external pure override returns (uint256) {
        return 42;
    }
}

// SHOULD PASS: Inherits from ISomething transitively through ISomethingExt.
interface ISomethingExt is ISomething {}

contract SomethingTransitive is ISomethingExt {
    function f1() external pure override returns (uint256) {
        return 7;
    }
}

// SHOULD FAIL: Abstract contract with bodies is itself a target.
abstract contract SomethingBase { //~NOTE: contract `SomethingBase` implements interface `ISomething`'s external API but does not explicitly inherit from it
    function f1() external virtual returns (uint256) {
        return 1;
    }
}

// SHOULD PASS: Inherited base `SomethingBase` already covers ISomething's selectors.
contract SomethingDerived is SomethingBase {
    function f1() external override returns (uint256) {
        return 2;
    }
}

// SHOULD FAIL: Implements both IERC20Like and IERC20LikeMetadata; only the maximal interface is reported.
contract Token { //~NOTE: contract `Token` implements interface `IERC20LikeMetadata`'s external API but does not explicitly inherit from it
    function transfer(address, uint256) external pure returns (bool) {
        return true;
    }
    function balanceOf(address) external pure returns (uint256) {
        return 0;
    }
    function name() external pure returns (string memory) {
        return "T";
    }
}

// SHOULD FAIL: Signature-only abstract `IFooLike` is treated as a candidate interface.
contract Foo { //~NOTE: contract `Foo` implements interface `IFooLike`'s external API but does not explicitly inherit from it
    function foo() external pure returns (uint256) {
        return 1;
    }
}

// SHOULD PASS: No external functions, nothing to flag.
contract Empty {
    uint256 internal x;
}

// SHOULD PASS: Implements only a strict subset of IERC20Like's selectors.
contract OnlyTransfer {
    function transfer(address, uint256) external pure returns (bool) {
        return true;
    }
}

// SHOULD PASS: Libraries are not analyzed as targets.
library SomeLib {
    function f1() external pure returns (uint256) {
        return 1;
    }
}

// SHOULD FAIL: Implements two unrelated interfaces; both are reported.
contract MultiNoInherit { //~NOTE: contract `MultiNoInherit` implements interface `ISomething`'s external API but does not explicitly inherit from it
    //~^NOTE: contract `MultiNoInherit` implements interface `IExtra`'s external API but does not explicitly inherit from it
    function f1() external pure returns (uint256) {
        return 1;
    }
    function g() external pure returns (uint256) {
        return 2;
    }
}

// SHOULD FAIL: Implements an interface declared in an external dependency without inheriting it.
contract External { //~NOTE: contract `External` implements interface `IExternalThing`'s external API but does not explicitly inherit from it
    function doExternalThing() external pure returns (uint256) {
        return 1;
    }
}

// SHOULD PASS: Explicitly inherits the external dependency interface.
contract ExternalExplicit is IExternalThing {
    function doExternalThing() external pure override returns (uint256) {
        return 1;
    }
}
