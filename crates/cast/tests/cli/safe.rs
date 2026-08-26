use alloy_consensus::Transaction;
use alloy_eips::Typed2718;
use alloy_primitives::{address, b256};
use alloy_provider::Provider;
use alloy_rpc_types::BlockNumberOrTag;
use anvil::NodeConfig;
use foundry_test_utils::util::OutputExt;

casttest!(safe_commands_are_exposed, |_prj, cmd| {
    let output =
        cmd.cast_fuse().args(["safe", "--help"]).assert_success().get_output().stdout_lossy();
    for command in [
        "create",
        "add-delegate",
        "list-delegates",
        "remove-delegate",
        "propose",
        "sign",
        "simulate",
        "execute",
    ] {
        assert!(output.contains(command), "expected `cast safe {command}` in help:\n{output}");
    }
});

casttest!(safe_signing_commands_support_hardware_wallets, |_prj, cmd| {
    for command in ["create", "add-delegate", "remove-delegate", "propose", "sign", "execute"] {
        let output = cmd
            .cast_fuse()
            .args(["safe", command, "--help"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(output.contains("--ledger"), "expected Ledger support in help:\n{output}");
        assert!(output.contains("--trezor"), "expected Trezor support in help:\n{output}");
    }
});

casttest!(safe_onchain_commands_support_tempo_transaction_options, |_prj, cmd| {
    for command in ["create", "execute"] {
        let output = cmd
            .cast_fuse()
            .args(["safe", command, "--help"])
            .assert_success()
            .get_output()
            .stdout_lossy();
        assert!(
            output.contains("--tempo.fee-token"),
            "expected Tempo fee-token support in help:\n{output}"
        );
        assert!(
            output.contains("--tempo.nonce-key"),
            "expected Tempo nonce-key support in help:\n{output}"
        );
    }
});

casttest!(safe_create_honors_transaction_options, async |_prj, cmd| {
    let (api, handle) = anvil::spawn(NodeConfig::test()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let singleton = address!("1111111111111111111111111111111111111111");
    let factory = address!("2222222222222222222222222222222222222222");

    api.anvil_set_code(singleton, "0x00".parse().unwrap()).await.unwrap();
    // mstore(singleton); emit ProxyCreation(proxy, singleton); return proxy.
    api.anvil_set_code(
        factory,
        "0x7311111111111111111111111111111111111111115f527333333333333333333333333333333333333333337f4f51faf6c4561ff95f067657e43439f0f856d97c04d9ec9070a6199ad418e23560205fa27333333333333333333333333333333333333333335f5260205ff3"
            .parse()
            .unwrap(),
    )
    .await
    .unwrap();

    let owner = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    let key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let singleton = singleton.to_string();
    let factory = factory.to_string();
    let common_args = [
        "safe",
        "create",
        owner,
        "--singleton",
        &singleton,
        "--factory",
        &factory,
        "--fallback-handler",
        "0x0000000000000000000000000000000000000000",
        "--private-key",
        key,
        "--rpc-url",
        &rpc,
    ];

    cmd.cast_fuse().args(common_args).arg("--legacy").assert_success();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transaction = block.transactions.as_transactions().unwrap().last().unwrap();
    assert_eq!(transaction.ty(), 0);

    cmd.cast_fuse()
        .args(common_args)
        .args([
            "--access-list",
            r#"[{"address":"0x4444444444444444444444444444444444444444","storageKeys":["0x5555555555555555555555555555555555555555555555555555555555555555"]}]"#,
        ])
        .assert_success();
    let block =
        provider.get_block_by_number(BlockNumberOrTag::Latest).full().await.unwrap().unwrap();
    let transaction = block.transactions.as_transactions().unwrap().last().unwrap();
    let access_list = transaction.access_list().expect("explicit access list");
    assert_eq!(access_list.len(), 1);
    assert_eq!(access_list[0].address, address!("4444444444444444444444444444444444444444"));
    assert_eq!(
        access_list[0].storage_keys,
        [b256!("5555555555555555555555555555555555555555555555555555555555555555")]
    );
});
