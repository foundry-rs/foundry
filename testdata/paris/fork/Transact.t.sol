// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IERC20 {
    event Transfer(address indexed from, address indexed to, uint256 value);
    function balanceOf(address account) external view returns (uint256);
}

contract TransactTest is Test {
    // Monad mainnet USDC
    address constant USDC = 0x754704Bc059F8C67012fEd69BC8A327a5aafb603;
    // Sender of the USDC transfer
    address constant SENDER = 0x65b1683fA503005EeF709613566F02cE8A621c26;
    // Recipient of the USDC transfer
    address constant RECIPIENT = 0x240c0AE518EAA5667670d79560F16Fe4D9949d52;
    // Transfer amount: 400 USDC (6 decimals)
    uint256 constant AMOUNT = 400000000;
    // Balances at block 38118706 (before the transfer)
    uint256 constant SENDER_BALANCE_BEFORE = 550671950;
    uint256 constant RECIPIENT_BALANCE_BEFORE = 0;

    /// forge-config: default.rpc_storage_caching.chains = ["monad"]
    function testTransact() public {
        // Fork at block 38118706
        vm.createSelectFork("monad", 38118706);

        // Verify balances before transact
        assertEq(IERC20(USDC).balanceOf(SENDER), SENDER_BALANCE_BEFORE);
        assertEq(IERC20(USDC).balanceOf(RECIPIENT), RECIPIENT_BALANCE_BEFORE);

        // Replay the USDC transfer transaction from block 38118707
        // tx: 0x62068873ff1d3681a117c13563584226126bccc22c4d1f47fc4367d475d9e824
        vm.transact(0x62068873ff1d3681a117c13563884226126bccc22c4d1f47fc4367d475d9e824);

        // Verify balances after transact
        assertEq(IERC20(USDC).balanceOf(SENDER), SENDER_BALANCE_BEFORE - AMOUNT);
        assertEq(IERC20(USDC).balanceOf(RECIPIENT), RECIPIENT_BALANCE_BEFORE + AMOUNT);
    }

    /// forge-config: default.rpc_storage_caching.chains = ["monad"]
    function testTransactCooperatesWithCheatcodes() public {
        // Fork at block 38118706
        vm.createSelectFork("monad", 38118706);

        // Verify balances before transact
        assertEq(IERC20(USDC).balanceOf(SENDER), SENDER_BALANCE_BEFORE);
        assertEq(IERC20(USDC).balanceOf(RECIPIENT), RECIPIENT_BALANCE_BEFORE);

        // Expect the Transfer event
        vm.expectEmit(true, true, false, true, USDC);
        emit IERC20.Transfer(SENDER, RECIPIENT, AMOUNT);

        // Start recording logs
        vm.recordLogs();

        // Replay the USDC transfer transaction
        vm.transact(0x62068873ff1d3681a117c13563884226126bccc22c4d1f47fc4367d475d9e824);

        // Verify the recorded logs contain the Transfer event
        Vm.Log[] memory logs = vm.getRecordedLogs();
        assertGt(logs.length, 0);

        // Find the Transfer event in logs
        bool foundTransfer = false;
        bytes32 transferTopic = keccak256("Transfer(address,address,uint256)");
        for (uint256 i = 0; i < logs.length; i++) {
            if (logs[i].topics[0] == transferTopic && logs[i].emitter == USDC) {
                foundTransfer = true;
                // Verify indexed parameters (from, to)
                assertEq(address(uint160(uint256(logs[i].topics[1]))), SENDER);
                assertEq(address(uint160(uint256(logs[i].topics[2]))), RECIPIENT);
                // Verify amount from data
                assertEq(abi.decode(logs[i].data, (uint256)), AMOUNT);
                break;
            }
        }
        assertTrue(foundTransfer, "Transfer event not found");

        // Verify balances after transact
        assertEq(IERC20(USDC).balanceOf(SENDER), SENDER_BALANCE_BEFORE - AMOUNT);
        assertEq(IERC20(USDC).balanceOf(RECIPIENT), RECIPIENT_BALANCE_BEFORE + AMOUNT);
    }
}
