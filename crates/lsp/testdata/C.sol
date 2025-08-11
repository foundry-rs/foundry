// SPDX-License-Identifier: MIT
pragma solidity ^0.8.29;

import {A} from "./A.sol";
import {B as D} from "./B.sol";

contract C is A {
    using D for *;

    uint256 immutable SCREAM = 124;

    D.State public votes;
    function() internal c;

    constructor() {
        votes.name = "2024 Elections";
        name("meek");
    }

    function add_vote(string memory name) public returns (uint256) {
        bool fad;
        name.add_one(votes);
        return name.get_votes(votes);
    }
}

contract E {}
