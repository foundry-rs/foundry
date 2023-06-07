// SPDX-License-Identifier: Unlicense
pragma solidity 0.8.18;

import "ds-test/test.sol";
import "./Cheats.sol";

contract AddrMock is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function exposed_addr(uint256 pk) public returns (address) {
        return cheats.addr(pk);
    }

}

contract AddrTest is DSTest {
    Cheats constant cheats = Cheats(HEVM_ADDRESS);

    function testRevertsPrivKeyZero() public {
        // Deploy a mock contract to test reverts
        AddrMock mock = new AddrMock();

        cheats.expectRevert();
        mock.exposed_addr(0);
    }

    function testAddr() public {
        uint256 pk = 77814517325470205911140941194401928579557062014761831930645393041380819009408;
        address expected = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;

        assertEq(cheats.addr(pk), expected, "expected address did not match");
    }
}
