// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "ds-test/test.sol";

/*//////////////////////////////////////////////////////////////
    Here we test that the fuzz engine can include a contract created during the fuzz
        in its fuzz dictionary and eventually break the invariant. 
    Specifically, can Judas, a created contract from Jesus, break Jesus contract
        by revealing his identity.
/*/
/////////////////////////////////////////////////////////////

contract Jesus {
    address fren;
    bool public identity_revealed;

    function create_fren() public {
        fren = address(new Judas());
    }

    function kiss() public {
        require(msg.sender == fren);
        identity_revealed = true;
    }
}

contract Judas {
    Jesus jesus;

    constructor() {
        jesus = Jesus(msg.sender);
    }

    function betray() public {
        jesus.kiss();
    }
}

contract InvariantInnerContract is DSTest {
    Jesus jesus;

    function setUp() public {
        jesus = new Jesus();
    }

    function invariantHideJesus() public {
        require(jesus.identity_revealed() == false, "jesus betrayed");
    }
}
