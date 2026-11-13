// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

abstract contract Base {
    error ZeroAddress();
    address public a;

    function __Base_init(address x) internal {
        require(x != address(0), ZeroAddress());
        _set(x);
    }

    function _set(address x) internal {
        a = x;
    }
}

contract Harness is Base {
    function init(address x) external {
        __Base_init(x);
    }
}
