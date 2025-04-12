// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.8.19 <0.9.0;

import "forge-std/Script.sol";

contract HelperContract {
    event AddressCalled(address caller, address thisAddr);

    function getOwnAddress() public {
        emit AddressCalled(msg.sender, address(this));
    }
}

contract ScriptAddressWarnTest is Script {
    event ScriptAddressCalled(address thisAddr);

    HelperContract internal helper;

    function setUp() public {
        helper = new HelperContract();
    }

    function run() public {
        // This should trigger the warning
        address scriptAddr = address(this);
        emit ScriptAddressCalled(scriptAddr);

        // This should NOT trigger the warning
        helper.getOwnAddress();

        vm.stopBroadcast();
    }
} 
