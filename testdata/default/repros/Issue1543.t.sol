// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

struct BeforeTestSelectors {
    bytes4 test_selector;
    bytes4[] before_selectors;
}

contract SelfDestructor {
    function kill() external {
        selfdestruct(payable(msg.sender));
    }
}

// https://github.com/foundry-rs/foundry/issues/1543
contract Issue1543Test is DSTest {
    SelfDestructor killer;
    uint256 a;
    uint256 b;

    function setUp() public {
        killer = new SelfDestructor();
    }

    function beforeTestSelectors() public pure returns (BeforeTestSelectors[] memory) {
        BeforeTestSelectors[] memory targets = new BeforeTestSelectors[](3);
        bytes4[] memory selectors = new bytes4[](1);
        selectors[0] = this.kill_contract.selector;
        targets[0] = BeforeTestSelectors(this.testKill.selector, selectors);

        selectors = new bytes4[](3);
        selectors[0] = this.testA.selector;
        selectors[1] = this.testA.selector;
        selectors[2] = this.testA.selector;
        targets[1] = BeforeTestSelectors(this.testA.selector, selectors);

        selectors = new bytes4[](1);
        selectors[0] = this.setB.selector;
        targets[2] = BeforeTestSelectors(this.testB.selector, selectors);

        return targets;
    }

    function kill_contract() external {
        uint256 killer_size = getSize(address(killer));
        require(killer_size == 106);
        killer.kill();
    }

    function testKill() public view {
        uint256 killer_size = getSize(address(killer));
        require(killer_size == 0);
    }

    function getSize(address c) public view returns (uint32) {
        uint32 size;
        assembly {
            size := extcodesize(c)
        }
        return size;
    }

    function testA() public {
        require(a <= 3);
        a += 1;
    }

    function testSimpleA() public view {
        require(a == 0);
    }

    function setB() public {
        b = 100;
    }

    function testB() public {
        require(b == 100);
    }
}
