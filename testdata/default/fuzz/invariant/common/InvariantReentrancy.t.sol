// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity 0.8.18;

import "ds-test/test.sol";

contract Malicious {
    function world() public {
        // add code so contract is accounted as valid sender
        // see https://github.com/foundry-rs/foundry/issues/4245
        payable(msg.sender).call("");
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

    // do not include `mal` in identified contracts
    // see https://github.com/foundry-rs/foundry/issues/4245
    function targetContracts() public view returns (address[] memory) {
        address[] memory targets = new address[](1);
        targets[0] = address(vuln);
        return targets;
    }

    function invariantNotStolen() public {
        require(vuln.stolen() == false, "stolen");
    }
}
