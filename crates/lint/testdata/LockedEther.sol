//@compile-flags: --only-lint locked-ether

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

interface IERC20 {
    function transfer(address, uint256) external returns (bool);
}

// SHOULD FAIL:

contract LockedReceive { //~WARN: contract can receive ETH but has no mechanism to send it out
    receive() external payable {}
}

contract LockedFallback { //~WARN: contract can receive ETH but has no mechanism to send it out
    fallback() external payable {}
}

contract LockedPayableFn { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}
}

contract LockedPayableCtor { //~WARN: contract can receive ETH but has no mechanism to send it out
    constructor() payable {}
}

contract LockedZeroValue { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function noop(address payable to) external {
        to.transfer(0);
        bool ok = to.send(0);
        ok;
        (bool s,) = to.call{value: 0}("");
        s;
    }
}

contract LockedTokenOnly { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}

    function rescueToken(address token, address to, uint256 amount) external {
        IERC20(token).transfer(to, amount);
    }
}

contract Helper {
    function pay(address payable to, uint256 amount) external {
        to.transfer(amount);
    }
}

contract LockedNotInherited { //~WARN: contract can receive ETH but has no mechanism to send it out
    function deposit() external payable {}
}

contract Child { //~WARN: contract can receive ETH but has no mechanism to send it out
    constructor() payable {}
}

// SHOULD PASS:

contract OkTransfer {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        to.transfer(amount);
    }
}

contract OkSend {
    receive() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        bool ok = to.send(amount);
        require(ok);
    }
}

contract OkCallWithValue {
    function deposit() external payable {}

    function withdraw(address to, uint256 amount) external {
        (bool ok,) = to.call{value: amount}("");
        require(ok);
    }
}

contract OkSelfdestruct {
    function deposit() external payable {}

    function close(address payable to) external {
        selfdestruct(to);
    }
}

contract OkNewWithValue {
    function deposit() external payable {}

    function spawn(uint256 amount) external {
        new Child{value: amount}();
    }
}

abstract contract Withdrawable {
    function _withdraw(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkInheritedWithdraw is Withdrawable {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        _withdraw(to, amount);
    }
}

contract OkTransitive {
    function deposit() external payable {}

    function withdraw(address payable to, uint256 amount) external {
        _doSend(to, amount);
    }

    function _doSend(address payable to, uint256 amount) internal {
        to.transfer(amount);
    }
}

contract OkDelegatecall {
    function deposit() external payable {}

    function delegate(address impl, bytes calldata data) external {
        (bool ok,) = impl.delegatecall(data);
        require(ok);
    }
}

contract NotPayable {
    function ping() external pure returns (bool) {
        return true;
    }
}

library OkLib {
    function helper() internal pure returns (uint256) {
        return 1;
    }
}

interface IOk {
    function payme() external payable;
}
