// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.0;

import "ds-test/test.sol";

contract Target {
    bool ownerFound;
    bool amountFound;
    bool magicFound;
    bool keyFound;
    bool backupFound;
    bool extraStringFound;

    function fuzzWithFixtures(
        address owner_,
        uint256 _amount,
        int32 magic,
        bytes32 key,
        bytes memory backup,
        string memory extra
    ) external {
        if (owner_ == address(0x6B175474E89094C44Da98b954EedeAC495271d0F)) {
            ownerFound = true;
        }
        if (_amount == 1122334455) amountFound = true;
        if (magic == -777) magicFound = true;
        if (key == "abcd1234") keyFound = true;
        if (keccak256(backup) == keccak256("qwerty1234")) backupFound = true;
        if (keccak256(abi.encodePacked(extra)) == keccak256(abi.encodePacked("112233aabbccdd"))) {
            extraStringFound = true;
        }
    }

    function isCompromised() public view returns (bool) {
        return ownerFound && amountFound && magicFound && keyFound && backupFound && extraStringFound;
    }
}

/// Try to compromise target contract by finding all accepted values using fixtures.
contract InvariantFixtures is DSTest {
    Target target;
    address[] public fixture_owner_ = [address(0x6B175474E89094C44Da98b954EedeAC495271d0F)];
    uint256[] public fixture_amount = [1, 2, 1122334455];

    function setUp() public {
        target = new Target();
    }

    function fixtureMagic() external returns (int32[2] memory) {
        int32[2] memory magic;
        magic[0] = -777;
        magic[1] = 777;
        return magic;
    }

    function fixtureKey() external pure returns (bytes32[] memory) {
        bytes32[] memory keyFixture = new bytes32[](1);
        keyFixture[0] = "abcd1234";
        return keyFixture;
    }

    function fixtureBackup() external pure returns (bytes[] memory) {
        bytes[] memory backupFixture = new bytes[](1);
        backupFixture[0] = "qwerty1234";
        return backupFixture;
    }

    function fixtureExtra() external pure returns (string[] memory) {
        string[] memory extraFixture = new string[](1);
        extraFixture[0] = "112233aabbccdd";
        return extraFixture;
    }

    function invariant_target_not_compromised() public {
        assertEq(target.isCompromised(), false);
    }
}
