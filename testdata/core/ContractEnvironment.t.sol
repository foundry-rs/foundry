// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract ContractEnvironmentTest is DSTest {
    function chainId() internal view returns (uint256 id) {
        assembly {
            id := chainid()
        }
    }

    function testAddresses() public {
        assertEq(msg.sender, 0x00a329c0648769A73afAc7F9381E08FB43dBEA72, "sender account is incorrect");
        assertEq(tx.origin, 0x00a329c0648769A73afAc7F9381E08FB43dBEA72, "origin account is incorrect");
        assertEq(address(this), 0xb4c79daB8f259C7Aee6E5b2Aa729821864227e84, "test contract address is incorrect");
    }

    function testEnvironment() public {
        assertEq(chainId(), 31337, "chainid is incorrect");
        assertEq(block.number, 1, "block number is incorrect");
        assertEq(
            blockhash(block.number),
                0x0,
            "blockhash is incorrect"
        );
        assertEq(block.coinbase, 0x0000000000000000000000000000000000000000, "coinbase is incorrect");
        assertEq(block.timestamp, 1, "timestamp is incorrect");
        assertEq(block.difficulty, 0, "difficulty is incorrect");
    }
}
