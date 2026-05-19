//@compile-flags: --only-lint costly-loop

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.13;

contract CostlyLoop {
    uint256 public counter;
    uint256[] public values;
    mapping(address => uint256) public balances;

    // bad: direct state variable write in loop
    function badIncrement(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            counter++;
        }
    }

    // bad: assignment to state variable in loop
    function badAssign(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            counter = i;
        }
    }

    // bad: state variable written in while loop
    function badWhile(uint256 n) public {
        uint256 i = 0;
        while (i < n) {
            counter += 1;
            i++;
        }
    }

    // bad: mapping write in loop
    function badMapping(address[] calldata users, uint256 amount) public {
        for (uint256 i = 0; i < users.length; i++) {
            balances[users[i]] = amount;
        }
    }

    // bad: array element write in loop
    function badArrayWrite(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            values[i] = i;
        }
    }

    // bad: delete state variable in loop
    function badDelete(uint256 n) public {
        for (uint256 i = 0; i < n; i++) {
            delete counter;
        }
    }

    // good: local variable written in loop, state written once after
    function goodLocal(uint256 n) public {
        uint256 local = counter;
        for (uint256 i = 0; i < n; i++) {
            local++;
        }
        counter = local;
    }

    // good: state variable only read in loop
    function goodRead(uint256 n) public view returns (uint256) {
        uint256 sum = 0;
        for (uint256 i = 0; i < n; i++) {
            sum += counter;
        }
        return sum;
    }

    // good: state variable written outside of any loop
    function goodOutsideLoop() public {
        counter = 42;
    }
}
