// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "./Cheats.sol";

contract EnvTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testSetEnv() public {
        cheats.setEnv("_foundryCheatcodeSetEnvTestKey", "_foundryCheatcodeSetEnvTestValue");
        assertEq(1, 0, "gg");
    }
}
