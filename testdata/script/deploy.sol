// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import {DSTest} from "../lib/ds-test/src/test.sol";
import {Cheats} from "../cheats/Cheats.sol";

contract Greeter {
    string name;
    uint256 age;

    event Greet(string greet);

    function greeting(string memory _name) public returns (string memory) {
        name = _name;
        string memory greet = string(abi.encodePacked("Hello ", _name));
        emit Greet(greet);
        return greet;
    }

    function setAge(uint256 _age) external {
        age = _age;
    }
}

contract Deploy is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    Greeter greeter;
    string greeting;

    function run() external {
        cheats.startBroadcast();
        greeter = new Greeter();
        greeting = greeter.greeting("john");
        greeter.setAge(123);
        cheats.stopBroadcast();
    }
}
