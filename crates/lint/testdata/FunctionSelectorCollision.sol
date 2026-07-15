//@compile-flags: --only-lint function-selector-collision

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IImplementation {
    function gsf() external;
    function shared() external;
}

// The fallback's typed target designates IImplementation as this proxy's implementation API.
contract TypedProxy { //~WARN: proxy function `TypedProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    // An identical signature is function shadowing, not a selector hash collision.
    function shared() external {}

    fallback() external payable {
        (bool success,) = address(implementation).delegatecall(msg.data);
        if (!success) revert();
    }
}

contract ConcreteImplementation {
    function collate_propagate_storage(bytes16) external {}
}

// Concrete contract types designate an implementation API too.
contract ConcreteTypedProxy { //~WARN: proxy function `ConcreteTypedProxy.burn(uint256)` collides with implementation function `ConcreteImplementation.collate_propagate_storage(bytes16)` at selector `0x42966c68`
    ConcreteImplementation internal immutable implementation;

    constructor(ConcreteImplementation implementation_) {
        implementation = implementation_;
    }

    function burn(uint256) external {}

    fallback() external payable {
        (bool success,) = payable(address(implementation)).delegatecall(msg.data);
        if (!success) revert();
    }
}

// Unrelated contracts are not compared without an explicit proxy/implementation relationship.
contract UnrelatedImplementation {
    function gsf() external {}
}

contract UnrelatedContract {
    function tgeo() external {}
}

// Address-only targets do not designate an implementation API and stay out of scope.
contract AddressProxy {
    address internal immutable implementation;

    constructor(address implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        (bool success,) = implementation.delegatecall(msg.data);
        if (!success) revert();
    }
}

// A typed delegatecall outside the fallback does not create proxy dispatch behavior.
contract TypedDelegatecallUser {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    function forward() external {
        (bool success,) = address(implementation).delegatecall(msg.data);
        if (!success) revert();
    }
}

// A fallback forwarding a fixed payload is not a general proxy fallback.
contract FixedPayloadDelegatecall {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        (bool success,) = address(implementation).delegatecall(
            abi.encodeCall(IImplementation.gsf, ())
        );
        if (!success) revert();
    }
}

// The parameterized fallback input is exactly equal to msg.data.
contract ParameterizedFallbackProxy { //~WARN: proxy function `ParameterizedFallbackProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Reassigned fallback calldata is no longer proven to be the full msg.data.
contract ReassignedFallbackInputProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        input = input[4:];
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

library DelegatecallExtension {
    function delegatecall(address, bytes calldata) internal pure returns (bool, bytes memory) {
        return (true, "");
    }
}

// A user-defined address member with the same name is not an EVM delegatecall.
contract UserDefinedDelegatecall {
    using DelegatecallExtension for address;

    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external {
        address(implementation).delegatecall(msg.data);
    }
}
