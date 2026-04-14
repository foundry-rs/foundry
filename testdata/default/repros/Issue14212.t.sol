// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.18;

import "utils/Test.sol";

// https://github.com/foundry-rs/foundry/issues/14212
// EthEvmNetwork uses Ethereum as its Network type, which cannot deserialize
// OP Stack deposit transactions (type 0x7e). These tests verify that the fork
// backend can handle blocks and transactions containing deposit txs.

contract Issue14212Test is Test {
    // Base block 30434326 contains a deposit tx at index 0:
    //   tx:   0x6fc82bcdcdeba0385c3910cd8e92074e51aaa9f21528dbc4c242f560a2f27bab
    //   type: 0x7e (deposit)
    //   from: 0xDeaDDEaDDeAdDeAdDEAdDEaddeAddEAdDEAd0001
    //   to:   0x4200000000000000000000000000000000000015
    //
    // A regular tx in the same block:
    //   tx:   0xe2f4bffbcc88dd94cabf9b15e2318df0afc2ec895012274d0ecec3d27d6da3e2

    /// vm.transact on an OP deposit tx should not revert with a deserialization error.
    /// This exercises the fork backend's get_transaction codepath.
    function test_transactDepositTxOnBase() public {
        // Fork Base at the block before the deposit tx
        vm.createSelectFork("base", 30434325);

        // Transact the deposit tx from the next block.
        // This calls fork.backend().get_transaction() which uses FEN::Network
        // to deserialize the response. With Network = Ethereum, this fails:
        //   "deserialization error: data did not match any variant of untagged enum BlockTransactions"
        vm.transact(0xe2f4bffbcc88dd94cabf9b15e2318df0afc2ec895012274d0ecec3d27d6da3e2);
    }

    /// vm.rollFork to a tx hash in a block containing deposit txs should work.
    /// This exercises the fork backend's get_full_block codepath.
    function test_rollForkToTxOnBase() public {
        vm.createSelectFork("base", 30434325);

        // Roll to a regular tx in block 30434326 which also contains deposit txs.
        // This calls get_full_block internally which must deserialize the entire
        // block including the deposit tx.
        bytes32 txHash = 0xe2f4bffbcc88dd94cabf9b15e2318df0afc2ec895012274d0ecec3d27d6da3e2;
        vm.rollFork(txHash);
    }

    /// vm.transact on an OP deposit tx on Optimism mainnet.
    function test_transactDepositTxOnOptimism() public {
        // Optimism block 127867197 contains deposit tx at index 0:
        //   tx: 0x56b04a2e66cba482270c6e68244a8faaa59a1e1878a04086a12514be2d6e14f9
        vm.createSelectFork("optimism", 127867196);

        // Transact a regular tx from the block containing deposit txs.
        // The regular tx at index 1:
        vm.transact(0xa003e419e2d7502269eb5eda56947b580120e00abfd5b5460d08f8af44a0c24f);
    }
}
