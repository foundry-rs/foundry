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
