// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract Counter {
    uint256 public n;

    function inc() external {
        n++;
    }
}

contract Poker {
    function poke(Counter counter) external {
        counter.inc();
    }
}

/// forge-config: default.isolate = true
contract IsolatedStaticCallWarmthTest is Test {
    Counter counter;
    Poker poker;

    function setUp() public {
        counter = new Counter();
        poker = new Poker();
    }

    function balanceGas(address account) internal view returns (uint256 gas) {
        uint256 value;
        gas = gasleft();
        assembly {
            value := balance(account)
        }
        gas -= gasleft();
        require(value != type(uint256).max);
    }

    // An isolated STATICCALL cools the running transaction's accounts as for a new transaction,
    // the test contract included.
    function testStaticCallCoolsTestContract() public {
        assertLt(balanceGas(address(this)), 2600);
        counter.n();
        assertGt(balanceGas(address(this)), 2600);
    }

    // An account the STATICCALL cooled stays cold for the calling frame after an isolated CALL
    // accesses it.
    function testStaticCallCooledAccountStaysColdAfterIsolatedCall() public {
        counter.n();
        poker.poke(counter);
        assertGt(balanceGas(address(counter)), 2600);
        assertLt(balanceGas(address(counter)), 2600);
    }
}
