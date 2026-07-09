//! CLI tests for shared Tempo transaction options.

use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, U256, hex};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_sol_types::{SolEvent, SolValue};
use anvil::NodeConfig;
use foundry_evm::core::tempo::PATH_USD_ADDRESS;
use foundry_test_utils::util::OutputExt;
use tempo_contracts::precompiles::{
    IReceivePolicyGuard, ITIP20, ITIP403Registry, TIP403_REGISTRY_ADDRESS,
};
use tempo_hardfork::TempoHardfork;

fn json_success_data(output: &str) -> serde_json::Value {
    let envelope: serde_json::Value =
        serde_json::from_str(output.trim()).expect("command emits JSON");
    assert_eq!(envelope["success"], true, "unexpected JSON envelope: {envelope}");
    envelope["data"].clone()
}

casttest!(tempo_state_changing_help_includes_expires, |_prj, cmd| {
    let cases: &[(&str, &[&str])] = &[
        ("batch-mktx", &["batch-mktx", "--help"]),
        ("batch-send", &["batch-send", "--help"]),
        ("keychain authorize", &["keychain", "authorize", "--help"]),
        ("tip20 create", &["tip20", "create", "--help"]),
        ("tip20 logo-set", &["tip20", "logo-set", "--help"]),
        ("tip20 mine", &["tip20", "mine", "--help"]),
        ("storage-credits set-mode", &["storage-credits", "set-mode", "--help"]),
        ("storage-credits set-budget", &["storage-credits", "set-budget", "--help"]),
        ("vaddr create", &["vaddr", "create", "--help"]),
    ];

    for (name, args) in cases {
        let output = cmd.cast_fuse().args(*args).assert_success().get_output().stdout_lossy();
        assert!(
            output.contains("--tempo.expires <SECONDS>"),
            "expected {name} help to expose --tempo.expires, got:\n{output}",
        );
    }
});

casttest!(receive_policy_receipt_json_and_claim_flow, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let receiver = accounts[1];
    let recovery = accounts[2];
    let claim_target = accounts[3];
    let recovery_wallet = handle.dev_wallets().nth(2).unwrap();
    assert_eq!(recovery_wallet.address(), recovery);
    let recovery_pk = format!("0x{}", hex::encode(recovery_wallet.credential().to_bytes()));
    let amount = U256::from(77_000u64);
    let path_usd = PATH_USD_ADDRESS.to_string();
    let sender_arg = sender.to_string();
    let receiver_arg = receiver.to_string();
    let recovery_arg = recovery.to_string();
    let claim_target_arg = claim_target.to_string();

    let warning_preview = cmd
        .cast_fuse()
        .args(["--json", "receive-policy", "set", "0", "1", "--preview", "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let warning_preview = json_success_data(&warning_preview);
    let warning = warning_preview["warning"].as_str().expect("warning should be present");
    assert!(warning.contains("originator recovery is enabled"), "{warning}");
    assert!(warning.contains("system/precompile sender"), "{warning}");

    let safe_preview = cmd
        .cast_fuse()
        .args([
            "--json",
            "receive-policy",
            "set",
            "0",
            "1",
            "--recovery-authority",
            &recovery_arg,
            "--preview",
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let safe_preview = json_success_data(&safe_preview);
    assert_eq!(safe_preview["action"], "set_receive_policy");
    assert_eq!(safe_preview["recovery_mode"], "authority");
    assert!(safe_preview["calldata"].as_str().is_some_and(|s| s.starts_with("0x")));
    assert!(safe_preview["warning"].is_null());

    let registry = ITIP403Registry::new(TIP403_REGISTRY_ADDRESS, &provider);
    let set_policy_tx = TransactionRequest::default()
        .from(receiver)
        .to(TIP403_REGISTRY_ADDRESS)
        .with_input(registry.setReceivePolicy(0, 1, recovery).calldata().clone())
        .with_gas_limit(10_000_000);
    let set_policy = provider
        .send_transaction(WithOtherFields::new(set_policy_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(set_policy.status(), "setReceivePolicy should succeed");

    let validate = cmd
        .cast_fuse()
        .args([
            "--json",
            "receive-policy",
            "validate",
            &path_usd,
            &sender_arg,
            &receiver_arg,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let validate = json_success_data(&validate);
    assert_eq!(validate["authorized"], false);
    assert_eq!(validate["blocked_reason"], "receive_policy");
    assert_eq!(validate["delivery_state"], "held");

    let token = ITIP20::new(PATH_USD_ADDRESS, &provider);
    let claim_target_before = token.balanceOf(claim_target).call().await.unwrap();
    let transfer_tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD_ADDRESS)
        .with_input(token.transfer(receiver, amount).calldata().clone())
        .with_gas_limit(10_000_000);
    let transfer = provider
        .send_transaction(WithOtherFields::new(transfer_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(transfer.status(), "blocked transfer should still succeed");

    let blocked = transfer
        .inner
        .logs()
        .iter()
        .find_map(|log| IReceivePolicyGuard::TransferBlocked::decode_log(&log.inner).ok())
        .expect("transfer should emit TransferBlocked");
    assert_eq!(blocked.token, PATH_USD_ADDRESS);
    assert_eq!(blocked.receiver, receiver);
    assert_eq!(blocked.amount, amount);
    let decoded = IReceivePolicyGuard::ClaimReceiptV1::abi_decode(&blocked.receipt).unwrap();
    assert_eq!(decoded.version, 1);
    assert_eq!(decoded.recoveryAuthority, recovery);
    assert_eq!(decoded.originator, sender);
    assert_eq!(decoded.recipient, receiver);
    assert_eq!(decoded.blockedReason, ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8);
    assert_eq!(decoded.memo, B256::ZERO);

    let receipt_arg = blocked.receipt.to_string();
    let decoded_output = cmd
        .cast_fuse()
        .args(["--json", "receive-policy", "receipt", "decode", &receipt_arg])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let decoded_output = json_success_data(&decoded_output);
    assert_eq!(decoded_output["receipt"], receipt_arg);
    assert_eq!(decoded_output["token"], path_usd);
    assert_eq!(decoded_output["recovery_mode"], "authority");
    assert_eq!(decoded_output["originator"], sender_arg);
    assert_eq!(decoded_output["recipient"], receiver_arg);
    assert_eq!(decoded_output["blocked_reason"], "receive_policy");
    assert_eq!(decoded_output["kind"], "transfer");
    assert_eq!(decoded_output["delivery_state"], "unknown");
    assert_eq!(decoded_output["claim_target"], receiver_arg);

    let human_decode = cmd
        .cast_fuse()
        .args(["receive-policy", "receipt", "decode", &receipt_arg])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(human_decode.contains("cast receive-policy claim"), "{human_decode}");
    assert!(human_decode.contains(&receipt_arg), "{human_decode}");

    let balance_output = cmd
        .cast_fuse()
        .args(["--json", "receive-policy", "receipt", "balance", &receipt_arg, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let balance_output = json_success_data(&balance_output);
    assert_eq!(balance_output["held_balance"], amount.to_string());
    assert_eq!(balance_output["delivery_state"], "held");

    cmd.cast_fuse()
        .args([
            "receive-policy",
            "claim",
            &claim_target_arg,
            &receipt_arg,
            "--private-key",
            &recovery_pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();

    assert_eq!(token.balanceOf(claim_target).call().await.unwrap(), claim_target_before + amount);

    let claimed_balance_output = cmd
        .cast_fuse()
        .args(["--json", "receive-policy", "receipt", "balance", &receipt_arg, "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let claimed_balance_output = json_success_data(&claimed_balance_output);
    assert_eq!(claimed_balance_output["held_balance"], "0");
    assert_eq!(claimed_balance_output["delivery_state"], "not_held");
});

// The ReceivePolicyGuard precompile is only active from T6, so claim/burn must fail early on a
// pre-T6 RPC instead of submitting a transaction that would silently succeed as a no-op.
casttest!(receive_policy_claim_and_burn_require_t6, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = format!("0x{}", hex::encode(wallet.credential().to_bytes()));

    let receipt = IReceivePolicyGuard::ClaimReceiptV1::new(
        PATH_USD_ADDRESS,
        Address::ZERO,
        wallet.address(),
        Address::with_last_byte(0xbe),
        1_780_000_000,
        1,
        ITIP403Registry::BlockedReason::RECEIVE_POLICY as u8,
        IReceivePolicyGuard::InboundKind::TRANSFER,
        B256::ZERO,
    )
    .abi_encode();
    let receipt_arg = format!("0x{}", hex::encode(&receipt));

    let claim_err = cmd
        .cast_fuse()
        .args([
            "receive-policy",
            "claim",
            &wallet.address().to_string(),
            &receipt_arg,
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        claim_err
            .contains("cast receive-policy claim requires a Tempo T6-capable ReceivePolicy RPC"),
        "{claim_err}"
    );

    let burn_err = cmd
        .cast_fuse()
        .args([
            "receive-policy",
            "receipt",
            "burn",
            &receipt_arg,
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(
        burn_err.contains(
            "cast receive-policy receipt burn requires a Tempo T6-capable ReceivePolicy RPC"
        ),
        "{burn_err}"
    );
});

// Exercises the full TIP-403 policy lifecycle: create, inspect, check, and modify membership.
casttest!(tip403_policy_lifecycle, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = format!("0x{}", hex::encode(wallet.credential().to_bytes()));
    let admin = wallet.address();
    let member = handle.dev_wallets().nth(1).unwrap().address();

    // IDs 0 and 1 are reserved, so the first user policy on a fresh node is ID 2.
    let create_err = cmd
        .cast_fuse()
        .args([
            "tip403",
            "create",
            "whitelist",
            "--admin",
            &admin.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success()
        .get_output()
        .stderr_lossy();
    assert!(create_err.contains("Expected policy ID: 2"), "{create_err}");

    let info = cmd
        .cast_fuse()
        .args(["--json", "tip403", "info", "2", "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    let info = json_success_data(&info);
    assert_eq!(info["exists"], true);
    assert_eq!(info["policy_type"], "whitelist");
    assert_eq!(info["admin"], admin.to_string());

    // Non-member is not authorized by a whitelist policy until added.
    let before = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&before)["authorized"], false);

    cmd.cast_fuse()
        .args([
            "tip403",
            "whitelist",
            "add",
            "2",
            &member.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();

    let after = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&after)["authorized"], true);

    // Built-in policies are labeled.
    let allow_all = cmd
        .cast_fuse()
        .args(["--json", "tip403", "info", "1", "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&allow_all)["builtin"], "allow-all");
});

casttest!(tip403_create_warns_on_virtual_member, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let pk =
        format!("0x{}", hex::encode(handle.dev_wallets().next().unwrap().credential().to_bytes()));

    // A TIP-1022 virtual address (bytes [4:14] == 0xFD) is rejected on-chain on T3+; cast warns
    // and lets the chain enforce rather than hard-failing client-side.
    let virtual_addr = "0x12345678fdfdfdfdfdfdfdfdfdfdaabbccdd0011";
    let err = cmd
        .cast_fuse()
        .args([
            "tip403",
            "create",
            "whitelist",
            "--admin",
            "0x0000000000000000000000000000000000000001",
            "--member",
            virtual_addr,
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(err.contains("looks like a TIP-1022 virtual address"), "{err}");
});

casttest!(tip403_blacklist_semantics, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = format!("0x{}", hex::encode(wallet.credential().to_bytes()));
    let admin = wallet.address();
    let member = handle.dev_wallets().nth(1).unwrap().address();

    cmd.cast_fuse()
        .args([
            "tip403",
            "create",
            "blacklist",
            "--admin",
            &admin.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();

    // A blacklist authorizes everyone until they are explicitly added.
    let before = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&before)["authorized"], true);

    cmd.cast_fuse()
        .args([
            "tip403",
            "blacklist",
            "add",
            "2",
            &member.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();
    let blocked = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&blocked)["authorized"], false);

    cmd.cast_fuse()
        .args([
            "tip403",
            "blacklist",
            "remove",
            "2",
            &member.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();
    let restored = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&restored)["authorized"], true);
});

casttest!(tip403_create_with_members, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = format!("0x{}", hex::encode(wallet.credential().to_bytes()));
    let admin = wallet.address();
    let member = handle.dev_wallets().nth(1).unwrap().address();

    // `--member` seeds the whitelist via createPolicyWithAccounts, so the member is authorized
    // immediately without a follow-up modify.
    cmd.cast_fuse()
        .args([
            "tip403",
            "create",
            "whitelist",
            "--admin",
            &admin.to_string(),
            "--member",
            &member.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();
    let check = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&check)["authorized"], true);
});

casttest!(tip403_works_pre_t6, async |_prj, cmd| {
    // TIP-403 is a Genesis precompile, so the base policy commands work before T6 activates.
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = format!("0x{}", hex::encode(wallet.credential().to_bytes()));
    let admin = wallet.address();
    let member = handle.dev_wallets().nth(1).unwrap().address();

    cmd.cast_fuse()
        .args([
            "tip403",
            "create",
            "whitelist",
            "--admin",
            &admin.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();
    let info = cmd
        .cast_fuse()
        .args(["--json", "tip403", "info", "2", "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&info)["policy_type"], "whitelist");

    cmd.cast_fuse()
        .args([
            "tip403",
            "whitelist",
            "add",
            "2",
            &member.to_string(),
            "--private-key",
            &pk,
            "--rpc-url",
            &rpc,
        ])
        .assert_success();
    let check = cmd
        .cast_fuse()
        .args(["--json", "tip403", "check", "2", &member.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&check)["authorized"], true);
});

casttest!(storage_credits_reads_and_writes, async |_prj, cmd| {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T7.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = format!("0x{}", hex::encode(wallet.credential().to_bytes()));
    let account = wallet.address();

    // A fresh account starts with no credits, the default `refund` mode, and a zero budget.
    let balance = cmd
        .cast_fuse()
        .args(["--json", "storage-credits", "balance", &account.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&balance)["balance"], 0);

    let mode = cmd
        .cast_fuse()
        .args(["--json", "storage-credits", "mode", &account.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&mode)["mode"], "refund");

    let budget = cmd
        .cast_fuse()
        .args(["--json", "storage-credits", "budget", &account.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&budget)["budget"], 0);

    // set-mode / set-budget send valid transactions that the precompile accepts.
    cmd.cast_fuse()
        .args(["storage-credits", "set-mode", "direct", "--private-key", &pk, "--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args(["storage-credits", "set-budget", "42", "--private-key", &pk, "--rpc-url", &rpc])
        .assert_success();

    // Mode and budget are transaction-local transient state (TIP-1060), so they reset to defaults
    // after the setter transaction ends; a standalone read never observes the previous write.
    let mode = cmd
        .cast_fuse()
        .args(["--json", "storage-credits", "mode", &account.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&mode)["mode"], "refund");

    let budget = cmd
        .cast_fuse()
        .args(["--json", "storage-credits", "budget", &account.to_string(), "--rpc-url", &rpc])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert_eq!(json_success_data(&budget)["budget"], 0);
});

casttest!(storage_credits_require_t7, async |_prj, cmd| {
    // The StorageCredits precompile only activates at T7, so reads must fail cleanly before then.
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let account = handle.dev_wallets().next().unwrap().address();

    let pk =
        format!("0x{}", hex::encode(handle.dev_wallets().next().unwrap().credential().to_bytes()));
    let expected = "requires a Tempo T7-capable StorageCredits RPC";

    let read_err = cmd
        .cast_fuse()
        .args(["storage-credits", "balance", &account.to_string(), "--rpc-url", &rpc])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(read_err.contains(expected), "{read_err}");

    // Writes must fail closed too; otherwise a pre-T7 send would look like a successful no-op.
    let set_mode_err = cmd
        .cast_fuse()
        .args(["storage-credits", "set-mode", "direct", "--private-key", &pk, "--rpc-url", &rpc])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(set_mode_err.contains(expected), "{set_mode_err}");

    let set_budget_err = cmd
        .cast_fuse()
        .args(["storage-credits", "set-budget", "1", "--private-key", &pk, "--rpc-url", &rpc])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(set_budget_err.contains(expected), "{set_budget_err}");
});

casttest!(tip20_logo_create_help_includes_logo_uri, |_prj, cmd| {
    let output = cmd
        .cast_fuse()
        .args(["tip20", "create", "--help"])
        .assert_success()
        .get_output()
        .stdout_lossy();

    assert!(
        output.contains("--logo-uri <URI>"),
        "expected tip20 create help to expose --logo-uri, got:\n{output}",
    );
});

casttest!(tip20_logo_commands_expose_browser_and_remote_sponsor_options, |_prj, cmd| {
    for args in [["tip20", "create", "--help"], ["tip20", "logo-set", "--help"]] {
        let output = cmd.cast_fuse().args(args).assert_success().get_output().stdout_lossy();
        assert!(output.contains("--browser"), "expected --browser in help, got:\n{output}");
        assert!(
            output.contains("--sponsor-url <URL>"),
            "expected --sponsor-url in help, got:\n{output}"
        );
    }
});

casttest!(tip20_logo_check_accepts_valid_values, |_prj, cmd| {
    for uri in ["", "https://example.com/logo.png", "HTTP://example.com/logo.png", "ipfs://token"] {
        cmd.cast_fuse().args(["tip20", "logo-check", uri]).assert_success();
    }
});

casttest!(tip20_logo_check_rejects_invalid_values, |_prj, cmd| {
    let invalid = cmd
        .cast_fuse()
        .args(["tip20", "logo-check", "ftp://example.com/logo.png"])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(invalid.contains("InvalidLogoURI"), "got:\n{invalid}");

    let too_long = format!("https://{}", "a".repeat(249));
    let output = cmd
        .cast_fuse()
        .args(["tip20", "logo-check", &too_long])
        .assert_failure()
        .get_output()
        .stderr_lossy();
    assert!(output.contains("LogoURITooLong"), "got:\n{output}");
});

casttest!(tip20_create_validates_logo_uri_before_network_setup, |_prj, cmd| {
    let output = cmd
        .cast_fuse()
        .args([
            "tip20",
            "create",
            "Logo Token",
            "LOGO",
            "USD",
            "0x0000000000000000000000000000000000000001",
            "0x0000000000000000000000000000000000000002",
            "0x0000000000000000000000000000000000000000000000000000000000000003",
            "--logo-uri",
            "ftp://example.com/logo.png",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(output.contains("client-side validation failed: InvalidLogoURI"), "got:\n{output}");
});

casttest!(tip20_logo_set_validates_logo_uri_before_network_setup, |_prj, cmd| {
    let output = cmd
        .cast_fuse()
        .args([
            "tip20",
            "logo-set",
            "0x0000000000000000000000000000000000000001",
            "ftp://example.com/logo.png",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(output.contains("client-side validation failed: InvalidLogoURI"), "got:\n{output}");
});
