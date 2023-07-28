// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.13;

import "forge-std/Test.sol";
import "../src/{contract_name}.sol"; 

contract {contract_name}Test is Test {
    {contract_name} public {instance_name};

    function setUp() public {
        {instance_name} = new {contract_name}();
    }
}