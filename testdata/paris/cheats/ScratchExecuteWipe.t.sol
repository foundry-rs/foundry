// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

contract ScratchExecuteWipeTest is Test {
    address constant SENDER = 0x5316812db67073C4d4af8BB3000C5B86c2877e94;
    address constant TARGET = 0x6Fd0A0CFF9A87aDF51695b40b4fA267855a8F4c6;
    // Transfers 17 wei from `SENDER` to `TARGET` at nonce 0 on chain 1.
    bytes constant RAW_TX =
        hex"f860806483030d40946fd0a0cff9a87adf51695b40b4fa267855a8f4c6118025a03ebeabbcfe43c2c982e99b376b5fb6e765059d7f215533c8751218cac99bbd80a00a56cf5c382442466770a756e81272d06005c9e90fb8dbc5b53af499d5aca856";

    function executeThenRevert() external {
        vm.store(TARGET, bytes32(uint256(2)), bytes32(uint256(4)));
        vm.executeTransaction(RAW_TX);
        revert("parent reverted");
    }

    /// forge-config: default.evm_version = "paris"
    function test_execute_wipe_in_transaction() public {
        vm.chainId(1);
        vm.deal(SENDER, 1 ether);
        vm.etch(TARGET, hex"6000ff");
        vm.store(TARGET, bytes32(0), bytes32(uint256(9)));
        try this.executeThenRevert() {} catch {}
        emit log_named_uint("wipe code", TARGET.code.length);
        emit log_named_uint("wipe slot0", uint256(vm.load(TARGET, bytes32(0))));
        emit log_named_uint("wipe slot2", uint256(vm.load(TARGET, bytes32(uint256(2)))));
        emit log_named_uint("wipe sender nonce", vm.getNonce(SENDER));
        emit log_named_uint("wipe target balance", TARGET.balance);
    }
}
