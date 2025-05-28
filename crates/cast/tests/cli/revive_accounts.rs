use foundry_test_utils::{casttest_serial, deploy_contract, revive::PolkadotNode, util::OutputExt};

casttest_serial!(test_cast_balance, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();
        let (account, _) = PolkadotNode::dev_accounts().next().expect("no dev accounts available");
        let account = account.to_string();

        cmd.cast_fuse().args(["balance", "--rpc-url", url, &account]).assert_success().stdout_eq(
            str![[r#"
999999900000000000000000000

"#]],
        );
    }
});

casttest_serial!(test_cast_nonce, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let url = PolkadotNode::http_endpoint();
        let (account, _) = PolkadotNode::dev_accounts().next().unwrap();
        let account = account.to_string();

        cmd.cast_fuse().args(["nonce", "--rpc-url", url, &account]).assert_success().stdout_eq(
            str![[r#"
0

"#]],
        );
    }
});

casttest_serial!(test_cast_code, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let (url, _deployer_pk, contract_address, _tx_hash) = deploy_contract!(cmd);

        cmd.cast_fuse()
            .args(["code", "--rpc-url", url, &contract_address])
            .assert_success()
            .stdout_eq(str![[r#"
0x5[..]

"#]]);
    }
});

casttest_serial!(test_cast_codesize, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let (url, _deployer_pk, contract_address, _tx_hash) = deploy_contract!(cmd);

        cmd.cast_fuse()
            .args(["codesize", "--rpc-url", url, &contract_address])
            .assert_success()
            .stdout_eq(str![[r#"
5501

"#]]);
    }
});

casttest_serial!(test_cast_storage, |_prj, cmd| {
    if let Ok(_node) = tokio::runtime::Runtime::new().unwrap().block_on(PolkadotNode::start()) {
        let (url, _deployer_pk, contract_address, _tx_hash) = deploy_contract!(cmd);

        cmd.cast_fuse()
            .args(["storage", "--rpc-url", url, &contract_address, "0x0"])
            .assert_success()
            .stdout_eq(str![[r#"
0x000000000000000000000000000000000000000000000000000000000000002a

"#]]);
    }
});
