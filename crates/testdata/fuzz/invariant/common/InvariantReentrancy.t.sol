// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract Malicious {
    function world() public {
        // Does not matter, since it will get overridden.
    }
}

contract Vulnerable {
    bool public open_door = false;
    bool public stolen = false;
    Malicious mal;

    constructor(address _mal) {
        mal = Malicious(_mal);
    }

    function hello() public {
        open_door = true;
        mal.world();
        open_door = false;
    }

    function backdoor() public {
        require(open_door, "");
        stolen = true;
    }
}

contract InvariantReentrancy is DSTest {
    Vulnerable vuln;
    Malicious mal;

    function setUp() public {
        mal = new Malicious();
        vuln = new Vulnerable(address(mal));
    }

    function invariantNotStolen() public {
        require(vuln.stolen() == false, "stolen.");
    }
}
