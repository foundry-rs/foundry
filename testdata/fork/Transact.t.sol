// SPDX-License-Identifier: Unlicense
pragma solidity >=0.8.0;

import "ds-test/test.sol";
import "../cheats/Cheats.sol";
import "../logs/console.sol";

contract TransactOnForkTest is DSTest {
    Cheats constant vm = Cheats(HEVM_ADDRESS);

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
}
