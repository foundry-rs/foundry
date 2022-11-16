//! Contains various tests related to forge script
use anvil::{spawn, NodeConfig};
use foundry_cli_test_utils::{
    forgetest_async,
    util::{TestCommand, TestProject},
    ScriptOutcome, ScriptTester,
};

forgetest_async!(
    can_deploy_multi_chain_script_without_lib,
    |prj: TestProject, cmd: TestCommand| async move {
        let (api1, handle1) = spawn(NodeConfig::test()).await;
        let (api2, handle2) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

        tester
            .load_private_keys(vec![0, 1])
            .await
            .add_sig("MultiChainBroadcastNoLink", "deploy(string memory,string memory)")
            .args(vec![handle1.http_endpoint(), handle2.http_endpoint()])
            .broadcast(ScriptOutcome::OkBroadcast);

        assert!(1 == api1.transaction_count(tester.accounts_pub[0], None).await.unwrap().as_u32());
        assert!(1 == api1.transaction_count(tester.accounts_pub[1], None).await.unwrap().as_u32());

        assert!(2 == api2.transaction_count(tester.accounts_pub[0], None).await.unwrap().as_u32());
        assert!(3 == api2.transaction_count(tester.accounts_pub[1], None).await.unwrap().as_u32());
    }
);

forgetest_async!(
    can_not_deploy_multi_chain_script_with_lib,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_, handle1) = spawn(NodeConfig::test()).await;
        let (_, handle2) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

        tester
            .load_private_keys(vec![0, 1])
            .await
            .add_deployer(0)
            .add_sig("MultiChainBroadcastLink", "deploy(string memory,string memory)")
            .args(vec![handle1.http_endpoint(), handle2.http_endpoint()])
            .broadcast(ScriptOutcome::UnsupportedLibraries);
    }
);

forgetest_async!(
    can_not_change_fork_during_broadcast,
    |prj: TestProject, cmd: TestCommand| async move {
        let (_, handle1) = spawn(NodeConfig::test()).await;
        let (_, handle2) = spawn(NodeConfig::test()).await;
        let mut tester = ScriptTester::new_broadcast_without_endpoint(cmd, prj.root());

        tester
            .load_private_keys(vec![0, 1])
            .await
            .add_deployer(0)
            .add_sig("MultiChainBroadcastNoLink", "deployError(string memory,string memory)")
            .args(vec![handle1.http_endpoint(), handle2.http_endpoint()])
            .broadcast(ScriptOutcome::ErrorSelectForkOnBroadcast);
    }
);
