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
}

contract AssertStateChangeExternal {
    IToken public token;
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
}
