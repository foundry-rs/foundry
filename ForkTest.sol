// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.1;

// cargo r --bin dapp test --contracts './ForkTest.sol' --fork-url <your url> --fork-block-number 13292582
contract ForkTest {
    // only passes at block https://etherscan.io/block/13292582
    function testBal() public {
        require(address(0x1aD91ee08f21bE3dE0BA2ba6918E714dA6B45836).balance == 5535797434271969477039);
    }
}
