//@compile-flags: --only-lint function-selector-collision

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IImplementation {
    function gsf() external;
    function forwarded() external;
    function shared() external;
}

interface ILoopSelectors {
    function f0() external;
    function f1() external;
    function f2() external;
    function f3() external;
    function f4() external;
    function f5() external;
    function f6() external;
    function f7() external;
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

// Mutating the fallback input after forwarding does not change the forwarded calldata.
contract ReassignedAfterForwardingProxy { //~WARN: proxy function `ReassignedAfterForwardingProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        input = input[4:];
        if (!success) revert();
        return returndata;
    }
}

// Call options execute before the delegatecall and can modify its calldata argument.
contract ReassignedInCallOptionsProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        (bool success, bytes memory returndata) = address(implementation).delegatecall{
            gas: (input = input[4:]).length
        }(input);
        if (!success) revert();
        return returndata;
    }
}

// Selector-correlated mutation does not suppress other unmodified selector paths.
contract SelectorCorrelatedMutationProxy { //~WARN: proxy function `SelectorCorrelatedMutationProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        if (msg.sig == IImplementation.forwarded.selector) input = input[4:];
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// A selector guard limits the implementation API reachable through this delegatecall.
contract SelectorGatedProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        if (msg.sig == IImplementation.forwarded.selector) {
            (bool success,) = address(implementation).delegatecall(msg.data);
            if (!success) revert();
        }
    }
}

// An inequality guard excludes its selector from the implementation API.
contract SelectorExcludedProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        if (msg.sig != IImplementation.gsf.selector) {
            (bool success,) = address(implementation).delegatecall(msg.data);
            if (!success) revert();
        }
    }
}

// A selector branch that exits cannot reach delegatecalls after the branch.
contract SelectorExitGatedProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        if (msg.sig == IImplementation.gsf.selector) revert();
        (bool success,) = address(implementation).delegatecall(msg.data);
        if (!success) revert();
    }
}

// An exiting try success clause does not make its catch clause unreachable.
contract TryCatchExitProxy { //~WARN: proxy function `TryCatchExitProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        try implementation.forwarded() {
            revert();
        } catch {
            (bool success,) = address(implementation).delegatecall(msg.data);
            if (!success) revert();
        }
    }
}

// Calldata mutation in a try success clause does not taint its catch path.
contract TryCatchMutationProxy { //~WARN: proxy function `TryCatchMutationProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        try implementation.forwarded() {
            input = input[4:];
        } catch {}
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// A while loop can fall through without executing an exiting body.
contract LoopZeroIterationProxy { //~WARN: proxy function `LoopZeroIterationProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        while (msg.value != 0) return;
        (bool success,) = address(implementation).delegatecall(msg.data);
        if (!success) revert();
    }
}

// Statements after break do not affect the loop's exit path.
contract LoopBreakProxy { //~WARN: proxy function `LoopBreakProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        while (true) {
            break;
            input = input[4:];
        }
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Statements after continue do not affect the loop's next iteration or exit path.
contract LoopContinueProxy { //~WARN: proxy function `LoopContinueProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external payable returns (bytes memory) {
        while (msg.value != 0) {
            continue;
            input = input[4:];
        }
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Continue in a do-while loop still evaluates the loop condition before exiting.
contract DoWhileContinueProxy { //~WARN: proxy function `DoWhileContinueProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        do {
            continue;
            input = input[4:];
        } while (false);
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Continue in a for loop still evaluates the next expression.
contract ForContinueProxy { //~WARN: proxy function `ForContinueProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external {
        for (
            ;
            msg.sig == IImplementation.gsf.selector;
            address(implementation).delegatecall(input)
        ) {
            continue;
        }
    }
}

// A false for-loop condition bypasses its body and next expression.
contract ForZeroIterationProxy { //~WARN: proxy function `ForZeroIterationProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        for (; false; input = input[4:]) {}
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Break in a for-loop body bypasses its next expression.
contract ForBreakProxy { //~WARN: proxy function `ForBreakProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        for (;; input = input[4:]) {
            break;
        }
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Break in a do-while body bypasses its condition.
contract DoWhileBreakProxy { //~WARN: proxy function `DoWhileBreakProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        do {
            break;
        } while ((input = input[4:]).length != 0);
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// A short-circuited operand does not mutate calldata on the skipped path.
contract ShortCircuitMutationProxy { //~WARN: proxy function `ShortCircuitMutationProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external payable returns (bytes memory) {
        if (msg.value != 0 || (input = input[4:]).length != 0) {}
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// A ternary mutation only affects the selected expression arm.
contract TernaryMutationProxy { //~WARN: proxy function `TernaryMutationProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external payable returns (bytes memory) {
        msg.value != 0 ? input = input[4:] : input;
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Only the mutated short-circuit path can fall through to the delegatecall.
contract ShortCircuitExitProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external payable returns (bytes memory) {
        if (msg.value != 0 || (input = input[4:]).length != 0) return "";
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// Only the mutated ternary path can fall through to the delegatecall.
contract TernaryExitProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external payable returns (bytes memory) {
        if (msg.value != 0 ? true : (input = input[4:]).length != 0) return "";
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// A selector constraint remains correlated through a short-circuit condition.
contract CompoundSelectorGatedProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        if (msg.sig == IImplementation.forwarded.selector && msg.value != 0) {
            (bool success,) = address(implementation).delegatecall(msg.data);
            if (!success) revert();
        }
    }
}

// A selector constraint remains correlated with its ternary arm's mutation.
contract TernarySelectorMutationProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback(bytes calldata input) external returns (bytes memory) {
        msg.sig == IImplementation.gsf.selector ? input = input[4:] : input;
        (bool success, bytes memory returndata) = address(implementation).delegatecall(input);
        if (!success) revert();
        return returndata;
    }
}

// An unreachable false loop body does not designate an implementation target.
contract FalseLoopProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external {
        while (false) {
            address(implementation).delegatecall(msg.data);
        }
    }
}

// An infinite true loop cannot fall through to a later delegatecall.
contract InfiniteLoopProxy {
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external {
        while (true) {}
        address(implementation).delegatecall(msg.data);
    }
}

// Loop state remains bounded when selector exclusions accumulate in different orders.
contract LoopStateBounded {
    fallback() external payable {
        while (msg.value != 0) {
            uint256 x = gasleft() % 8;
            if (x == 0) {
                if (msg.sig == ILoopSelectors.f0.selector) break;
            } else if (x == 1) {
                if (msg.sig == ILoopSelectors.f1.selector) break;
            } else if (x == 2) {
                if (msg.sig == ILoopSelectors.f2.selector) break;
            } else if (x == 3) {
                if (msg.sig == ILoopSelectors.f3.selector) break;
            } else if (x == 4) {
                if (msg.sig == ILoopSelectors.f4.selector) break;
            } else if (x == 5) {
                if (msg.sig == ILoopSelectors.f5.selector) break;
            } else if (x == 6) {
                if (msg.sig == ILoopSelectors.f6.selector) break;
            } else {
                if (msg.sig == ILoopSelectors.f7.selector) break;
            }
        }
    }
}

// A reachable selector collision is still reported through a selector guard.
contract CollisionSelectorGatedProxy { //~WARN: proxy function `CollisionSelectorGatedProxy.tgeo()` collides with implementation function `IImplementation.gsf()` at selector `0x67e43e43`
    IImplementation internal immutable implementation;

    constructor(IImplementation implementation_) {
        implementation = implementation_;
    }

    function tgeo() external {}

    fallback() external payable {
        if (IImplementation.gsf.selector == msg.sig) {
            (bool success,) = address(implementation).delegatecall(msg.data);
            if (!success) revert();
        }
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
