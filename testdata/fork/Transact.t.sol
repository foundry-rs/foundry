// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";
import "../logs/console.sol";

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);

    function balanceOf(address account) external view returns (uint256);
}

contract TransactOnForkTest is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

    IERC20 constant USDT = IERC20(0xdAC17F958D2ee523a2206206994597C13D831ec7);

    event Transfer(address indexed from, address indexed to, uint256 value);

    function testTransact() public {
        // A random block https://etherscan.io/block/15596646
        uint256 fork = vm.createFork("rpcAlias", 15596646);
        vm.selectFork(fork);
        // a random transfer transaction in the block: https://etherscan.io/tx/0xaba74f25a17cf0d95d1c6d0085d6c83fb8c5e773ffd2573b99a953256f989c89
        bytes32 tx = 0xaba74f25a17cf0d95d1c6d0085d6c83fb8c5e773ffd2573b99a953256f989c89;

        address sender = address(0xa98218cdc4f63aCe91ddDdd24F7A580FD383865b);
        address recipient = address(0x0C124046Fa7202f98E4e251B50488e34416Fc306);

        assertEq(sender.balance, 5764124000000000);
        assertEq(recipient.balance, 3936000000000000);

        // transfer amount: 0.000336 Ether
        uint256 transferAmount = 3936000000000000;
        uint256 expectedRecipientBalance = recipient.balance + transferAmount;
        uint256 expectedSenderBalance = sender.balance - transferAmount;

        // execute the transaction
        vm.transact(tx);

        // recipient received transfer
        assertEq(recipient.balance, expectedRecipientBalance);

        // decreased by transferAmount and gas
        assert(sender.balance < expectedSenderBalance);
    }

    function testTransactCooperatesWithCheatcodes() public {
        // A random block https://etherscan.io/block/16260609
        uint256 fork = vm.createFork("rpcAlias", 16260609);
        vm.selectFork(fork);

        // a random ERC20 USDT transfer transaction in the next block: https://etherscan.io/tx/0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c
        bytes32 tx = 0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c;

        address sender = address(0x2e09BB78B3D64d98Da44D1C776fa77dcd133ED54);
        address recipient = address(0x23a6B9711B711b1d404F2AA740bde350c67a6F06);

        uint256 senderBalance = USDT.balanceOf(sender);
        uint256 recipientBalance = USDT.balanceOf(recipient);

        assertEq(senderBalance, 20041000000);
        assertEq(recipientBalance, 66000000);

        // transfer amount: 14000 USDT
        uint256 transferAmount = 14000000000;
        uint256 expectedRecipientBalance = recipientBalance + transferAmount;
        uint256 expectedSenderBalance = senderBalance - transferAmount;

        // expect a call to USDT's transfer
        vm.expectCall(address(USDT), abi.encodeWithSelector(IERC20.transfer.selector, recipient, transferAmount));

        // expect a Transfer event to be emitted
        vm.expectEmit(true, true, false, true, address(USDT));
        emit Transfer(address(sender), address(recipient), transferAmount);

        // start recording logs
        vm.recordLogs();

        // execute the transaction
        vm.transact(tx);

        // extract recorded logs
        Cheats.Log[] memory logs = vm.getRecordedLogs();

        senderBalance = USDT.balanceOf(sender);
        recipientBalance = USDT.balanceOf(recipient);

        // recipient received transfer
        assertEq(recipientBalance, expectedRecipientBalance);

        // decreased by transferAmount
        assertEq(senderBalance, expectedSenderBalance);

        // recorded a `Transfer` log
        assertEq(logs.length, 1);
    }
}
