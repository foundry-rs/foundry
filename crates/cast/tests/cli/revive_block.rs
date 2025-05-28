use foundry_test_utils::{casttest_serial, revive::PolkadotNode, util::OutputExt};

casttest_serial!(test_cast_find_block, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();

        let latest_block_number = cmd
            .cast_fuse()
            .args(["block-number", "--rpc-url", url])
            .assert_success()
            .get_output()
            .stdout_lossy()
            .trim()
            .parse::<u64>()
            .expect("Failed to parse block number");

        let latest_block = cmd
            .cast_fuse()
            .args(["block", "latest", "--rpc-url", url, "--json"])
            .assert_success()
            .get_output()
            .stdout_lossy();

        let block: alloy_rpc_types::Block =
            serde_json::from_str(&latest_block).expect("Failed to parse block data");

        let ts = block.header.timestamp.to_string();
        let found_block = cmd
            .cast_fuse()
            .args(["find-block", &ts, "--rpc-url", url])
            .assert_success()
            .get_output()
            .stdout_lossy()
            .trim()
            .parse::<u64>()
            .expect("Failed to parse found block number");

        // The found block should be the same as or very close to the latest block
        assert!(
            found_block <= latest_block_number,
            "find-block({ts}) returned {found_block}, which is > latest block-number ({latest_block_number})"
        );
    }
});

casttest_serial!(test_cast_block_number, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();
        cmd.cast_fuse().args(["block-number", "--rpc-url", url]).assert_success().stdout_eq(str![
            [r#"
1

"#]
        ]);
    }
});

casttest_serial!(test_cast_gas_price, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();

        cmd.cast_fuse().args(["gas-price", "--rpc-url", url]).assert_success().stdout_eq(str![[
            r#"
1000

"#
        ]]);
    }
});

casttest_serial!(test_cast_basefee, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();

        cmd.cast_fuse().args(["basefee", "--rpc-url", url]).assert_success().stdout_eq(str![[r#"
1000

"#]]);
    }
});

casttest_serial!(test_cast_block, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();

        cmd.cast_fuse().args(["block", "latest", "--rpc-url", url]).assert_success().stdout_eq(
            str![[r#"


baseFeePerGas        1000
difficulty           0
extraData            0x
gasLimit             [..]
gasUsed              0
hash                 0x[..]
logsBloom            0x[..]
miner                0x[..]
mixHash              0x[..]
nonce                0x[..]
number               [..]
parentHash           0x[..]
parentBeaconRoot     
transactionsRoot     0x[..]
receiptsRoot         0x[..]
sha3Uncles           0x[..]
size                 0
stateRoot            0x[..]
timestamp            [..]
withdrawalsRoot      
totalDifficulty      
blobGasUsed          
excessBlobGas        
requestsHash         
transactions:        []

"#]],
        );
    }
});

casttest_serial!(test_cast_age, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();

        cmd.cast_fuse().args(["age", "latest", "--rpc-url", url]).assert_success().stdout_eq(str![
            [r#"
[..]

"#]
        ]);
    }
});
