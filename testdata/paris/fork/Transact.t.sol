// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

interface IERC20 {
    function transfer(address to, uint256 amount) external returns (bool);

    function balanceOf(address account) external view returns (uint256);
}

contract CallerRecorder {
    address public caller;

    function recordCaller() external {
        caller = msg.sender;
    }
}

contract TransactOnForkTest is Test {
    IERC20 constant USDT = IERC20(0xdAC17F958D2ee523a2206206994597C13D831ec7);
    bytes constant ALWAYS_REVERT = hex"60006000fd";

    event Transfer(address indexed from, address indexed to, uint256 value);

    function testTransact() public {
        // A random block https://etherscan.io/block/17134913
        uint256 fork = vm.createFork("mainnet", 17134913);
        vm.selectFork(fork);
        // a random transfer transaction in the next block: https://etherscan.io/tx/0xaf6201d435b216a858c580e20512a16136916d894aa33260650e164e3238c771
        bytes32 transaction = 0xaf6201d435b216a858c580e20512a16136916d894aa33260650e164e3238c771;

        address sender = address(0x9B315A70FEe05a70A9F2c832E93a7095FEb32Bfe);
        address recipient = address(0xDB358B93157Df9b3B1eE9Ea5CDB7D0aE9a1D8110);

        assertEq(sender.balance, 110231651357268209);
        assertEq(recipient.balance, 892860016357511);

        // transfer amount: 0.015 Ether
        uint256 transferAmount = 15000000000000000;
        uint256 expectedRecipientBalance = recipient.balance + transferAmount;
        uint256 expectedSenderBalance = sender.balance - transferAmount;

        // execute the transaction
        vm.transact(transaction);

        // recipient received transfer
        assertEq(recipient.balance, expectedRecipientBalance);

        // decreased by transferAmount and gas
        assert(sender.balance < expectedSenderBalance);
    }

    function testTransactCooperatesWithCheatcodes() public {
        // A random block https://etherscan.io/block/16260609
        uint256 fork = vm.createFork("mainnet", 16260609);
        vm.selectFork(fork);

        // a random ERC20 USDT transfer transaction in the next block: https://etherscan.io/tx/0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c
        bytes32 transaction = 0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c;

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
        // With the current expect call behavior, in which we expect calls to be matched in the next call's subcalls,
        // expecting calls on vm.transact is impossible. This is because transact essentially creates another call context
        // that operates independently of the current one, meaning that depths won't match and will trigger a panic on REVM,
        // as the transact storage is not persisted as well and can't be checked.
        // vm.expectCall(address(USDT), abi.encodeWithSelector(IERC20.transfer.selector, recipient, transferAmount));

        // expect a Transfer event to be emitted
        vm.expectEmit(true, true, false, true, address(USDT));
        emit Transfer(address(sender), address(recipient), transferAmount);

        // start recording logs
        vm.recordLogs();

        // execute the transaction
        vm.transact(transaction);

        // extract recorded logs
        Vm.Log[] memory logs = vm.getRecordedLogs();

        senderBalance = USDT.balanceOf(sender);
        recipientBalance = USDT.balanceOf(recipient);

        // recipient received transfer
        assertEq(recipientBalance, expectedRecipientBalance);

        // decreased by transferAmount
        assertEq(senderBalance, expectedSenderBalance);

        // recorded a `Transfer` log
        assertEq(logs.length, 1);
    }

    function testTransactUsesEtchedCode() public {
        vm.createSelectFork("mainnet", 16260609);
        vm.etch(address(USDT), ALWAYS_REVERT);
        vm.recordLogs();

        vm.transact(0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);

        assertEq(vm.getRecordedLogs().length, 0);
        assertEq(address(USDT).code, ALWAYS_REVERT);
    }

    function testTransactUsesPersistentEtchedCode() public {
        vm.createSelectFork("mainnet", 16260609);
        vm.etch(address(USDT), ALWAYS_REVERT);
        vm.makePersistent(address(USDT));
        vm.recordLogs();

        vm.transact(0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);

        assertEq(vm.getRecordedLogs().length, 0);
        assertEq(address(USDT).code, ALWAYS_REVERT);
    }

    function testSequentialTransactsPreservePersistentChanges() public {
        vm.createSelectFork("mainnet", 16260609);
        vm.makePersistent(address(USDT));
        address sender = address(0x2e09BB78B3D64d98Da44D1C776fa77dcd133ED54);
        address recipient = address(0xe688b84b23f322a994A53dbF8E15FA82CDB71127);
        uint256 recipientBalance = recipient.balance;

        vm.transact(0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);
        assertEq(USDT.balanceOf(sender), 6041000000);
        vm.transact(0x1dd8ba92b071798e39d7075f230b89d30ccab4b519fee5416bcb997578460c1a);

        assertEq(recipient.balance, recipientBalance + 0x42eec20fa78d6b);
        assertEq(USDT.balanceOf(sender), 6041000000);
    }

    function testTransactOnInactiveForkUsesTargetState() public {
        uint256 active = vm.createSelectFork("mainnet", 16260608);
        uint256 inactive = vm.createFork("mainnet", 16260609);
        address account = address(0xF00D);
        vm.selectFork(inactive);
        uint256 targetBalance = account.balance;
        bytes32 targetCodeHash = keccak256(address(USDT).code);
        vm.selectFork(active);
        vm.deal(account, targetBalance + 1);
        vm.etch(address(USDT), ALWAYS_REVERT);

        vm.transact(inactive, 0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);

        vm.selectFork(inactive);
        assertEq(account.balance, targetBalance);
        assertEq(keccak256(address(USDT).code), targetCodeHash);
    }

    function testTransactOnInactiveForkUsesPersistentState() public {
        uint256 active = vm.createSelectFork("mainnet", 16260608);
        uint256 inactive = vm.createFork("mainnet", 16260609);
        address account = address(0xF00D);
        vm.selectFork(inactive);
        uint256 targetBalance = account.balance;
        vm.selectFork(active);
        vm.deal(account, targetBalance + 1);
        vm.etch(address(USDT), ALWAYS_REVERT);
        vm.makePersistent(account);
        vm.makePersistent(address(USDT));

        vm.transact(inactive, 0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);

        vm.selectFork(inactive);
        assertEq(account.balance, targetBalance + 1);
        assertEq(address(USDT).code, ALWAYS_REVERT);
    }

    function testTransactOnInactiveForkDoesNotConsumePrank() public {
        vm.createSelectFork("mainnet", 16260608);
        uint256 inactive = vm.createFork("mainnet", 16260609);
        CallerRecorder recorder = new CallerRecorder();
        vm.makePersistent(address(recorder));
        address alice = address(0xA11CE);

        vm.prank(alice);
        vm.transact(inactive, 0x33350512fec589e635865cbdb38fa3a20a2aa160c52611f1783d0ba24ad13c8c);
        recorder.recordCaller();

        assertEq(recorder.caller(), alice);
    }
}
