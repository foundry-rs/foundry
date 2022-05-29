// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract EnvTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testSetEnv() public {
        string memory key = "_foundryCheatcodeSetEnvTestKey";
        string memory val = "_foundryCheatcodeSetEnvTestVal";
        cheats.setEnv(key, val);
    }
}
