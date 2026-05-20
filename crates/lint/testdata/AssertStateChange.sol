//@compile-flags: --only-lint assert-state-change

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

contract AssertStateChange {
    uint256 public counter;
    bool public flag;
    mapping(address => uint256) public balances;
    uint256[] public items;

    // Bad: pre-increment of state variable inside assert
    function badPreIncrement(uint256 expected) external {
        assert(++counter == expected); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: post-increment of state variable inside assert
    function badPostIncrement(uint256 expected) external {
        assert(counter++ == expected); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: call to state-mutating internal function
    function _toggleFlag() internal returns (bool) {
        flag = !flag;
        return flag;
    }

    function badMutatingCall() external {
        assert(_toggleFlag()); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: call to another state-mutating function
    function _deposit() internal returns (bool) {
        balances[msg.sender] += msg.value;
        return true;
    }

    function badDeposit() external payable {
        assert(_deposit()); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: state variable assignment inside assert
    function badAssignment(uint256 val) external {
        assert((counter = val) > 0); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: mapping index assignment (state variable lvalue)
    function badMappingAssign(address user, uint256 amt) external {
        assert((balances[user] = amt) > 0); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: pure comparison, no state change
    function goodComparison(uint256 expected) external view {
        assert(counter == expected);
    }

    // Good: view function call inside assert
    function _getCounter() internal view returns (uint256) {
        return counter;
    }

    function goodViewCall(uint256 expected) external view {
        assert(_getCounter() == expected);
    }

    // Good: require() with increment is fine (not assert)
    function goodRequire(uint256 expected) external {
        require(++counter == expected, "mismatch");
    }

    // Good: local variable increment, not a state variable
    function goodLocalInc() external pure returns (uint256) {
        uint256 local = 0;
        assert(++local == 1);
        return local;
    }

    // Good: local variable assignment, not a state variable
    function goodLocalAssign(uint256 val) external pure {
        uint256 local;
        assert((local = val) > 0);
    }
}

interface IToken {
    function transfer(address to, uint256 amount) external returns (bool);
    function balanceOf(address account) external view returns (uint256);
    // Overloaded: one mutating (2 args), one view (1 arg) — different arities so arity
    // narrowing can correctly distinguish them.
    function update(address target, uint256 amount) external returns (bool);
    function update(uint256 n) external view returns (bool);
}

// Interface with view functions that share names with low-level address builtins.
// These must NOT be flagged, the name-only heuristic must not fire when the receiver
// resolves to a known contract/interface.
interface IRouter {
    function send(uint256 amount) external view returns (bool);
    function call(bytes calldata data) external view returns (bool);
    function transfer(uint256 amount) external view returns (bool);
}

contract AssertStateChangeExternal {
    IToken public token;
    IRouter public router;
    address payable public recipient;

    // Bad: .send() always transfers ether (state-changing), returns bool
    function badSend() external {
        assert(recipient.send(1 ether)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Bad: interface call to a non-view function
    function badInterfaceCall(address to, uint256 amt) external {
        assert(token.transfer(to, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view function on an interface does not mutate state
    function goodInterfaceView(uint256 expected) external view {
        assert(token.balanceOf(address(this)) >= expected);
    }

    // Good: calling the 1-arg view overload `update(uint256)` — arity differs from
    // the 2-arg mutating overload, so no false positive.
    function goodOverloadView(uint256 n) external view {
        assert(token.update(n));
    }

    // Bad: calling the 2-arg mutating overload `update(address,uint256)` — only
    // overload with this arity, so correctly flagged.
    function badOverloadMutating(address target, uint256 amt) external {
        assert(token.update(target, amt)); //~WARN: assert() argument contains a state-modifying expression
    }

    // Good: view functions on an interface that happen to be named send/call/transfer
    // must NOT trigger the name-only address heuristic (fix for false positives).
    function goodViewSend(uint256 n) external view {
        assert(router.send(n));
    }

    function goodViewCall(bytes calldata data) external view {
        assert(router.call(data));
    }

    function goodViewTransfer(uint256 n) external view {
        assert(router.transfer(n));
    }
}

// ---- push/pop on contract/interface state vars must not be flagged ----
// These are interface method calls, not builtin array mutations. The name-only heuristic
// must not fire; resolve_member_overloads handles them precisely via mutates_state().
interface IQueue {
    function pop() external view returns (bool);
    function push(uint256 x) external view returns (bool);
}

contract AssertStateChangePushPop {
    IQueue public q;

    function goodInterfacePop() external view {
        assert(q.pop());
    }

    function goodInterfacePush(uint256 x) external view {
        assert(q.push(x));
    }
}

// ---- using-for library extension calls ----
// Solar does not yet embed Res on Member expressions for extension methods, so a dedicated
// library-scan fallback is required (fix for false negatives on using-for mutations).

library StorageLib {
    function bump(uint256[] storage arr) internal returns (bool) {
        arr.push(1);
        return true;
    }

    function peek(uint256[] storage arr) internal view returns (uint256) {
        return arr.length;
    }
}

contract AssertStateChangeUsingFor {
    using StorageLib for uint256[];

    uint256[] public items;

    // Bad: bump() writes to storage via a using-for library extension, must be flagged.
    function badLibraryExtension() external returns (bool) {
        assert(items.bump()); //~WARN: assert() argument contains a state-modifying expression
        return true;
    }

    // Good: peek() is a view extension — must NOT be flagged.
    function goodLibraryView() external view returns (bool) {
        assert(items.peek() >= 0);
        return true;
    }
}

// ---- library-fallback receiver-type guard ----
library OtherStorageLib {
    function bump(bytes storage b) internal returns (bool) {
        b.push(0x00);
        return true;
    }
}

contract AssertStateChangeUnrelatedLib {
    uint256[] public items;

    // Good: `items.bump()` has no matching extension (OtherStorageLib.bump takes bytes, not
    // uint256[]), and `uint256[]` has no member `bump()`. Must NOT be flagged.
    function goodUnrelatedLib() external view returns (bool) {
        return items.length == 0;
    }
}
