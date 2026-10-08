//! CLI tests for shared Tempo transaction options.

use alloy_network::{ReceiptResponse, TransactionBuilder};
use alloy_primitives::{Address, B256, U256, address, hex, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types::TransactionRequest;
use alloy_serde::WithOtherFields;
use alloy_sol_types::{SolEvent, SolValue};
use anvil::NodeConfig;
use foundry_cli::utils::parse_json;
use foundry_evm::core::tempo::PATH_USD_ADDRESS;
use foundry_test_utils::util::OutputExt;
use tempo_contracts::precompiles::{
    CURRENT_COMMITTEE_ADDRESS, ICurrentCommittee, IReceivePolicyGuard, IRolesAuth, ITIP20,
    ITIP20Factory, ITIP403Registry, TIP20_CHANNEL_RESERVE_ADDRESS, TIP20_FACTORY_ADDRESS,
    TIP403_REGISTRY_ADDRESS,
};
use tempo_hardfork::TempoHardfork;

fn json_success_data(output: &str) -> serde_json::Value {
    let envelope: serde_json::Value = parse_json(output.trim()).expect("command emits JSON");
    assert_eq!(envelope["success"], true, "unexpected JSON envelope: {envelope}");
    envelope["data"].clone()
}

#[casttest]
fn tempo_state_changing_help_includes_expires(cmd: _) {
    let cases: &[(&str, &[&str])] = &[
        ("batch-mktx", &["batch-mktx", "--help"]),
        ("batch-send", &["batch-send", "--help"]),
        ("keychain authorize", &["keychain", "authorize", "--help"]),
        ("tip20 create", &["tip20", "create", "--help"]),
        ("tip20 logo-set", &["tip20", "logo-set", "--help"]),
        ("tip20 mine", &["tip20", "mine", "--help"]),
        ("tip20 grant-role", &["tip20", "grant-role", "--help"]),
        ("tip20 revoke-role", &["tip20", "revoke-role", "--help"]),
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
}

#[casttest]
async fn receive_policy_receipt_json_and_claim_flow(cmd: _) {
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
    let recovery_pk = hex::encode_prefixed(recovery_wallet.credential().to_bytes());
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
}

// The ReceivePolicyGuard precompile is only active from T6, so claim/burn must fail early on a
// pre-T6 RPC instead of submitting a transaction that would silently succeed as a no-op.
#[casttest]
async fn receive_policy_claim_and_burn_require_t6(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode_prefixed(wallet.credential().to_bytes());

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
    let receipt_arg = hex::encode_prefixed(&receipt);

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
}

// Exercises the full TIP-403 policy lifecycle: create, inspect, check, and modify membership.
#[casttest]
async fn tip403_policy_lifecycle(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode_prefixed(wallet.credential().to_bytes());
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
}

#[casttest]
async fn tip403_create_warns_on_virtual_member(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let pk = hex::encode_prefixed(handle.dev_wallets().next().unwrap().credential().to_bytes());

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
}

#[casttest]
async fn tip403_blacklist_semantics(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode_prefixed(wallet.credential().to_bytes());
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
}

#[casttest]
async fn tip403_create_with_members(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode_prefixed(wallet.credential().to_bytes());
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
}

#[casttest]
async fn tip403_works_pre_t6(cmd: _) {
    // TIP-403 is a Genesis precompile, so the base policy commands work before T6 activates.
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode_prefixed(wallet.credential().to_bytes());
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
}

#[casttest]
async fn storage_credits_reads_and_writes(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T7.into()))).await;
    let rpc = handle.http_endpoint();
    let wallet = handle.dev_wallets().next().unwrap();
    let pk = hex::encode_prefixed(wallet.credential().to_bytes());
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
}

#[casttest]
async fn storage_credits_require_t7(cmd: _) {
    // The StorageCredits precompile only activates at T7, so reads must fail cleanly before then.
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T6.into()))).await;
    let rpc = handle.http_endpoint();
    let account = handle.dev_wallets().next().unwrap().address();

    let pk = hex::encode_prefixed(handle.dev_wallets().next().unwrap().credential().to_bytes());
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
}

#[casttest]
async fn current_committee_cast_run_decoding(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T8.into()))).await;
    let provider = handle.http_provider();
    let caller = handle.dev_accounts().next().unwrap();
    let committee = ICurrentCommittee::new(CURRENT_COMMITTEE_ADDRESS, &provider);
    let public_key = B256::repeat_byte(0x11);

    let getter = TransactionRequest::default()
        .from(caller)
        .to(CURRENT_COMMITTEE_ADDRESS)
        .with_input(committee.getCommitteeMembers().calldata().clone())
        .with_gas_limit(1_000_000);
    let getter_receipt = provider
        .send_transaction(WithOtherFields::new(getter))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(getter_receipt.status());

    cmd.cast_fuse();
    cmd.env("FOUNDRY_HARDFORK", "tempo:T8");
    let getter_stdout = cmd
        .args([
            "run",
            &getter_receipt.transaction_hash.to_string(),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(getter_stdout.contains("CurrentCommittee::getCommitteeMembers()"), "{getter_stdout}");
    assert!(getter_stdout.contains("← [Return] 0, []"), "{getter_stdout}");

    let setter = TransactionRequest::default()
        .from(caller)
        .to(CURRENT_COMMITTEE_ADDRESS)
        .with_input(committee.setCommitteeMembers(1, vec![public_key]).calldata().clone())
        .with_gas_limit(1_000_000);
    let setter_receipt = provider
        .send_transaction(WithOtherFields::new(setter))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(!setter_receipt.status());

    cmd.cast_fuse();
    cmd.env("FOUNDRY_HARDFORK", "tempo:T8");
    let setter_stdout = cmd
        .args([
            "run",
            &setter_receipt.transaction_hash.to_string(),
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();
    assert!(setter_stdout.contains("CurrentCommittee::setCommitteeMembers(1"), "{setter_stdout}");
    assert!(setter_stdout.contains("← [Revert] Unauthorized()"), "{setter_stdout}");
}

#[casttest]
fn tip20_logo_create_help_includes_logo_uri(cmd: _) {
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
}

#[casttest]
fn tip20_logo_commands_expose_browser_and_remote_sponsor_options(cmd: _) {
    for args in [["tip20", "create", "--help"], ["tip20", "logo-set", "--help"]] {
        let output = cmd.cast_fuse().args(args).assert_success().get_output().stdout_lossy();
        assert!(output.contains("--browser"), "expected --browser in help, got:\n{output}");
        assert!(
            output.contains("--sponsor-url <URL>"),
            "expected --sponsor-url in help, got:\n{output}"
        );
    }
}

#[casttest]
fn mktx_rejects_remote_sponsor_instead_of_ignoring_it(cmd: _) {
    let stderr = cmd
        .cast_fuse()
        .args([
            "mktx",
            "0x0000000000000000000000000000000000000001",
            "--private-key",
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
            "--sponsor-url",
            "http://localhost:1",
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("--sponsor-url is not supported by cast mktx"), "{stderr}");
}

#[casttest]
async fn send_with_presigned_sponsor_signature_keeps_digest_stable(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let sponsor = accounts[1];
    let filler = accounts[2];
    let recipient = accounts[3];
    let sender_wallet = handle.dev_wallets().next().unwrap();
    assert_eq!(sender_wallet.address(), sender);
    let sender_pk = hex::encode_prefixed(sender_wallet.credential().to_bytes());
    let sponsor_wallet = handle.dev_wallets().nth(1).unwrap();
    assert_eq!(sponsor_wallet.address(), sponsor);
    let sponsor_pk = hex::encode_prefixed(sponsor_wallet.credential().to_bytes());
    let token = PATH_USD_ADDRESS.to_string();
    let recipient_arg = recipient.to_string();
    let sponsor_arg = sponsor.to_string();

    // The sponsor digest commits to the full transaction, so nonce, gas limit and fees must be
    // pinned for `cast mktx` and `cast send` to build the identical transaction. The gas limit
    // must cover actual usage: a Tempo transaction that runs out of gas during AA validation is
    // dropped without a receipt.
    let nonce = provider.get_transaction_count(sender).await.unwrap().to_string();
    let pinned = [
        "--nonce",
        nonce.as_str(),
        "--gas-limit",
        "1000000",
        "--gas-price",
        "40gwei",
        "--priority-gas-price",
        "1gwei",
    ];

    let mut mktx_args = vec![
        "mktx",
        &token,
        "transfer(address,uint256)",
        &recipient_arg,
        "1",
        "--private-key",
        &sender_pk,
        "--rpc-url",
        &rpc,
        "--tempo.print-sponsor-hash",
        "--tempo.sponsor",
        &sponsor_arg,
    ];
    mktx_args.extend(pinned);
    let hash = cmd
        .cast_fuse()
        .args(&mktx_args)
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();
    assert!(hash.starts_with("0x") && hash.len() == 66, "unexpected sponsor hash: {hash}");

    let sig = cmd
        .cast_fuse()
        .args(["wallet", "sign", "--private-key", &sponsor_pk, "--no-hash", &hash])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // Mine an unrelated transaction between hash generation and send; the pinned fields must
    // keep the digest stable across the chain-state change.
    let tip20 = ITIP20::new(PATH_USD_ADDRESS, &provider);
    let filler_tx = TransactionRequest::default()
        .from(filler)
        .to(PATH_USD_ADDRESS)
        .with_input(tip20.transfer(recipient, U256::from(5u64)).calldata().clone())
        .with_gas_limit(10_000_000);
    let filler_receipt = provider
        .send_transaction(WithOtherFields::new(filler_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(filler_receipt.status(), "filler transfer should succeed");

    let mut send_args = vec![
        "send",
        &token,
        "transfer(address,uint256)",
        &recipient_arg,
        "1",
        "--private-key",
        &sender_pk,
        "--rpc-url",
        &rpc,
        "--tempo.sponsor",
        &sponsor_arg,
        "--tempo.sponsor-signature",
        &sig,
        "--json",
    ];
    send_args.extend(pinned);
    let send_assert = cmd.cast_fuse().args(&send_args).assert_success();
    let output = send_assert.get_output();

    let stderr = output.stderr_lossy();
    assert!(
        stderr.to_lowercase().contains(&format!("tempo sponsor digest: {}", hash.to_lowercase())),
        "sponsor digest drifted from the pre-signed hash {hash}:\n{stderr}"
    );

    let receipt: serde_json::Value =
        parse_json(output.stdout_lossy().trim()).expect("receipt should be JSON");
    assert_eq!(receipt["status"], "0x1", "unexpected receipt: {receipt}");
    let fee_payer: Address =
        receipt["feePayer"].as_str().expect("receipt has feePayer").parse().unwrap();
    assert_eq!(fee_payer, sponsor, "receipt fee payer should be the sponsor");
}

#[casttest]
async fn send_with_presigned_sponsor_signature_rejects_stale_digest(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let sponsor = accounts[1];
    let recipient = accounts[2];
    let sender_wallet = handle.dev_wallets().next().unwrap();
    assert_eq!(sender_wallet.address(), sender);
    let sender_pk = hex::encode_prefixed(sender_wallet.credential().to_bytes());
    let sponsor_wallet = handle.dev_wallets().nth(1).unwrap();
    assert_eq!(sponsor_wallet.address(), sponsor);
    let sponsor_pk = hex::encode_prefixed(sponsor_wallet.credential().to_bytes());
    let token = PATH_USD_ADDRESS.to_string();
    let recipient_arg = recipient.to_string();
    let sponsor_arg = sponsor.to_string();

    // Without pinned fields, both commands fill nonce and fees from live chain state.
    let hash = cmd
        .cast_fuse()
        .args([
            "mktx",
            &token,
            "transfer(address,uint256)",
            &recipient_arg,
            "1",
            "--private-key",
            &sender_pk,
            "--rpc-url",
            &rpc,
            "--tempo.print-sponsor-hash",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    let sig = cmd
        .cast_fuse()
        .args(["wallet", "sign", "--private-key", &sponsor_pk, "--no-hash", &hash])
        .assert_success()
        .get_output()
        .stdout_lossy()
        .trim()
        .to_string();

    // Consume the sender nonce so the refilled transaction no longer matches the signed digest.
    let tip20 = ITIP20::new(PATH_USD_ADDRESS, &provider);
    let bump_tx = TransactionRequest::default()
        .from(sender)
        .to(PATH_USD_ADDRESS)
        .with_input(tip20.transfer(recipient, U256::ONE).calldata().clone())
        .with_gas_limit(10_000_000);
    let bump_receipt = provider
        .send_transaction(WithOtherFields::new(bump_tx))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(bump_receipt.status(), "nonce bump transfer should succeed");

    let stderr = cmd
        .cast_fuse()
        .args([
            "send",
            &token,
            "transfer(address,uint256)",
            &recipient_arg,
            "1",
            "--private-key",
            &sender_pk,
            "--rpc-url",
            &rpc,
            "--tempo.sponsor",
            &sponsor_arg,
            "--tempo.sponsor-signature",
            &sig,
        ])
        .assert_failure()
        .get_output()
        .stderr_lossy();

    assert!(stderr.contains("Tempo sponsor signature recovered"), "{stderr}");
    assert!(stderr.contains("--tempo.print-sponsor-hash"), "{stderr}");
}

#[casttest]
async fn send_with_sponsor_url_uses_anvil_builtin_fee_payer(cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo()).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let accounts: Vec<Address> = handle.dev_accounts().collect();
    let sender = accounts[0];
    let sponsor = *accounts.last().unwrap();
    let recipient = accounts[3];
    let sender_wallet = handle.dev_wallets().next().unwrap();
    assert_eq!(sender_wallet.address(), sender);
    let sender_pk = hex::encode_prefixed(sender_wallet.credential().to_bytes());
    let token = PATH_USD_ADDRESS.to_string();
    let recipient_arg = recipient.to_string();
    let amount = U256::from(1000u64);

    let tip20 = ITIP20::new(PATH_USD_ADDRESS, &provider);
    let sender_before = tip20.balanceOf(sender).call().await.unwrap();
    let sponsor_before = tip20.balanceOf(sponsor).call().await.unwrap();

    // No local sponsor key or address: anvil's built-in fee payer signs the sponsorship request
    // via `eth_signRawTransaction`, so the sponsor URL is simply the node itself.
    let stdout = cmd
        .cast_fuse()
        .args([
            "send",
            &token,
            "transfer(address,uint256)",
            &recipient_arg,
            "1000",
            "--private-key",
            &sender_pk,
            "--rpc-url",
            &rpc,
            "--sponsor-url",
            &rpc,
            "--json",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let receipt: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("receipt should be JSON");
    assert_eq!(receipt["status"], "0x1", "unexpected receipt: {receipt}");
    assert_eq!(
        receipt["feePayer"],
        serde_json::to_value(sponsor).unwrap(),
        "fee payer should be anvil's default sponsor (last dev account): {receipt}"
    );
    assert_eq!(
        receipt["feeToken"],
        serde_json::to_value(PATH_USD_ADDRESS).unwrap(),
        "sponsor pays with its stored fee token: {receipt}"
    );

    let sender_after = tip20.balanceOf(sender).call().await.unwrap();
    let sponsor_after = tip20.balanceOf(sponsor).call().await.unwrap();
    assert_eq!(
        sender_after,
        sender_before - amount,
        "sender must only pay the transfer amount, fees are sponsored"
    );
    assert!(sponsor_after < sponsor_before, "sponsor must pay the transaction fee");
}

#[casttest]
fn tip20_logo_check_accepts_valid_values(cmd: _) {
    for uri in ["", "https://example.com/logo.png", "HTTP://example.com/logo.png", "ipfs://token"] {
        cmd.cast_fuse().args(["tip20", "logo-check", uri]).assert_success();
    }
}

#[casttest]
fn tip20_logo_check_rejects_invalid_values(cmd: _) {
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
}

#[casttest]
fn tip20_create_validates_logo_uri_before_network_setup(cmd: _) {
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
}

#[casttest]
fn tip20_logo_set_validates_logo_uri_before_network_setup(cmd: _) {
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
}

#[casttest]
async fn channel_id_defaults(cmd: _) {
    let (_api, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T5.into()))).await;
    let provider = handle.http_provider();
    let chain_id = provider.get_chain_id().await.unwrap();

    let payer = address!("0000000000000000000000000000000000000101");
    let payee = address!("0000000000000000000000000000000000000202");
    let salt = B256::with_last_byte(0x42);
    let expected = keccak256(
        (
            payer,
            payee,
            Address::ZERO,
            PATH_USD_ADDRESS,
            salt,
            Address::ZERO,
            B256::ZERO,
            TIP20_CHANNEL_RESERVE_ADDRESS,
            U256::from(chain_id),
        )
            .abi_encode(),
    );

    cmd.args([
        "channel-id",
        &payer.to_string(),
        &payee.to_string(),
        &PATH_USD_ADDRESS.to_string(),
        &salt.to_string(),
        "--rpc-url",
        handle.http_endpoint().as_str(),
    ])
    .assert_success()
    .stdout_eq(format!("{expected:#x}\n"));
}

#[casttest]
fn tempo_options_reject_conflicting_network(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.networks = foundry_evm_networks::NetworkVariant::Ethereum.into();
    });
    for command in ["access-list", "estimate", "send", "mktx", "call"] {
        cmd.cast_fuse()
            .current_dir(prj.root())
            .args([
                command,
                "0x0000000000000000000000000000000000000001",
                "--tempo.fee-token",
                "0x20c0000000000000000000000000000000000000",
                "--rpc-url",
                "http://127.0.0.1:1",
            ])
            .assert_failure()
            .stderr_eq(str![[r#"
Error: Tempo transaction options conflict with configured network `ethereum`

"#]]);
    }
}

#[casttest]
fn tempo_sessions_reject_conflicting_network(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.networks = foundry_evm_networks::NetworkVariant::Ethereum.into();
    });
    for command in ["access-list", "estimate", "send", "mktx", "call"] {
        cmd.cast_fuse()
            .current_dir(prj.root())
            .args([
                command,
                "0x0000000000000000000000000000000000000001",
                "--tempo.session",
                "0x4444444444444444444444444444444444444444444444444444444444444444",
                "--rpc-url",
                "http://127.0.0.1:1",
            ])
            .assert_failure()
            .stderr_eq(str![[r#"
Error: Tempo transaction options conflict with configured network `ethereum`

"#]]);
    }
}

#[casttest]
async fn tempo_mktx_selects_network_without_tempo_options(prj: _, cmd: _) {
    let (_, handle) = anvil::spawn(NodeConfig::test_tempo().with_chain_id(Some(4217u64))).await;
    let rpc = handle.http_endpoint();
    for network in [
        None,
        Some(foundry_evm_networks::NetworkVariant::Tempo),
        Some(foundry_evm_networks::NetworkVariant::Ethereum),
    ] {
        prj.update_config(|config| {
            config.networks = network.map(Into::into).unwrap_or_default();
        });
        let expected = if network == Some(foundry_evm_networks::NetworkVariant::Ethereum) {
            str![[r#"
0x02[..]

"#]]
        } else {
            str![[r#"
0x76[..]

"#]]
        };
        cmd.cast_fuse()
            .current_dir(prj.root())
            .args([
                "mktx",
                "0x0000000000000000000000000000000000000001",
                "--private-key",
                "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
                "--rpc-url",
                &rpc,
            ])
            .assert_success()
            .stdout_eq(expected);
    }
}

// A local Tempo node runs on chain 31337; once Tempo is selected, fee tokens must resolve as they
// do on a canonical Tempo chain ID.
#[casttest]
async fn tempo_selected_network_ignores_local_chain_id(prj: _, cmd: _) {
    prj.update_config(|config| {
        config.networks = foundry_evm_networks::NetworkVariant::Tempo.into();
    });
    let private_key = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let sponsor_key =
        "private-key://0x2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6";
    for chain_id in [Some(4217u64), None] {
        let (_, handle) = anvil::spawn(NodeConfig::test_tempo().with_chain_id(chain_id)).await;
        let rpc = handle.http_endpoint();
        let mktx = ["mktx", "0x0000000000000000000000000000000000000001", "--private-key"];

        cmd.cast_fuse()
            .current_dir(prj.root())
            .args(mktx)
            .args([private_key, "--rpc-url", &rpc])
            .assert_success()
            .stdout_eq(str![[r#"
0x76[..]

"#]])
            .stderr_eq(str![[r#"
Paying gas in AlphaUSD (0x20C0000000000000000000000000000000000001)

"#]]);

        cmd.cast_fuse()
            .current_dir(prj.root())
            .args(mktx)
            .args([private_key, "--rpc-url", &rpc])
            .args(["--tempo.sponsor", "0xa0Ee7A142d267C1f36714E4a8F75612F20a79720"])
            .args(["--tempo.sponsor-signer", sponsor_key])
            .assert_success()
            .stdout_eq(str![[r#"
0x76[..]

"#]])
            .stderr_eq(str![[r#"
Tempo sponsor: 0xa0Ee7A142d267C1f36714E4a8F75612F20a79720
Tempo fee token: 0x20C0000000000000000000000000000000000000
Tempo validity: after none, before none
Tempo sponsor digest: 0x[..]

"#]]);
    }
}

#[casttest]
fn tempo_zone_rejects_zero_amount(cmd: _) {
    for args in [
        vec!["tempo", "zone", "deposit", "--portal", "0x1111111111111111111111111111111111111111"],
        vec!["tempo", "zone", "withdraw", "--zone-id", "42", "--zone-chain-id", "1337"],
    ] {
        cmd.cast_fuse()
            .args(args)
            .args(["--amount", "0"])
            .assert_failure()
            .stdout_eq("")
            .stderr_eq(str![[r#"
Error: amount must be greater than zero

"#]]);
    }
}

#[casttest]
fn tempo_zone_rejects_callback_without_gas(cmd: _) {
    cmd.args([
        "tempo",
        "zone",
        "withdraw",
        "--zone-id",
        "7",
        "--zone-chain-id",
        "421700007",
        "--amount",
        "1",
        "--callback-data",
        "0x1234",
    ])
    .assert_failure()
    .stdout_eq("")
    .stderr_eq(str![[r#"
Error: --callback-data requires a nonzero --callback-gas-limit

"#]]);
}

/// Returns the address and private key of the dev account at `index`.
fn dev_account(handle: &anvil::NodeHandle, index: usize) -> (Address, String) {
    let wallet = handle.dev_wallets().nth(index).unwrap();
    (wallet.address(), hex::encode_prefixed(wallet.credential().to_bytes()))
}

/// Creates a TIP-20 token administered by the first dev account and returns its address.
async fn create_role_test_token(
    cmd: &mut foundry_test_utils::TestCommand,
    handle: &anvil::NodeHandle,
) -> Address {
    let (admin, admin_pk) = dev_account(handle, 0);
    let salt = B256::with_last_byte(1);
    cmd.cast_fuse()
        .args([
            "tip20",
            "create",
            "Role Test",
            "ROLE",
            "USD",
            &PATH_USD_ADDRESS.to_string(),
            &admin.to_string(),
            &salt.to_string(),
            "--private-key",
            &admin_pk,
            "--rpc-url",
            &handle.http_endpoint(),
        ])
        .assert_success();
    ITIP20Factory::new(TIP20_FACTORY_ADDRESS, handle.http_provider())
        .getTokenAddress(admin, salt)
        .call()
        .await
        .unwrap()
}

/// Expected `cast --json tip20 has-role` output.
fn has_role_json(
    token: Address,
    role: B256,
    role_name: Option<&str>,
    account: Address,
    has_role: bool,
) -> String {
    serde_json::json!({
        "schema_version": 1,
        "success": true,
        "data": {
            "token": token.to_string(),
            "role": role.to_string(),
            "role_name": role_name,
            "account": account.to_string(),
            "has_role": has_role,
        },
        "errors": [],
        "warnings": [],
    })
    .to_string()
}

// A freshly created token only assigns `DEFAULT_ADMIN_ROLE`, so minting is gated on granting
// `ISSUER_ROLE` and stops working again once the role is revoked.
#[casttest]
async fn tip20_issuer_role_grant_and_revoke_gate_minting(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T12.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let (admin, admin_pk) = dev_account(&handle, 0);
    let token = create_role_test_token(&mut cmd, &handle).await;
    let token_arg = token.to_string();
    let admin_arg = admin.to_string();
    let tip20 = ITIP20::new(token, &provider);
    let issuer_role = tip20.ISSUER_ROLE().call().await.unwrap();

    cmd.cast_fuse()
        .args(["tip20", "has-role", &token_arg, "admin", &admin_arg, "--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
Token:    0x20c000000000000000000000A3C1274aaDd82e4D
Role:     DEFAULT_ADMIN_ROLE (0x0000000000000000000000000000000000000000000000000000000000000000)
Account:  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
Has role: true

"#]]);
    cmd.cast_fuse()
        .args(["--json", "tip20", "has-role", &token_arg, "issuer", &admin_arg, "--rpc-url", &rpc])
        .assert_json_stdout(has_role_json(token, issuer_role, Some("ISSUER_ROLE"), admin, false));

    let mint = ["erc20", "mint", &token_arg, &admin_arg, "1000", "--private-key", &admin_pk];
    cmd.cast_fuse().args(mint).args(["--rpc-url", &rpc]).assert_failure().stderr_eq(str![[r#"
Error: Failed to estimate gas: server returned an error response: error code 3: execution reverted: custom error 0x82b42900, data: "0x82b42900": Unauthorized

"#]]);

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "issuer", &admin_arg, "--private-key", &admin_pk])
        .args(["--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args(["--json", "tip20", "has-role", &token_arg, "ISSUER_ROLE", &admin_arg])
        .args(["--rpc-url", &rpc])
        .assert_json_stdout(has_role_json(token, issuer_role, Some("ISSUER_ROLE"), admin, true));

    cmd.cast_fuse().args(mint).args(["--rpc-url", &rpc]).assert_success();
    assert_eq!(tip20.balanceOf(admin).call().await.unwrap(), U256::from(1000));

    cmd.cast_fuse()
        .args([
            "tip20",
            "revoke-role",
            &token_arg,
            "issuer",
            &admin_arg,
            "--private-key",
            &admin_pk,
        ])
        .args(["--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args(["--json", "tip20", "has-role", &token_arg, "issuer", &admin_arg, "--rpc-url", &rpc])
        .assert_json_stdout(has_role_json(token, issuer_role, Some("ISSUER_ROLE"), admin, false));
    cmd.cast_fuse().args(mint).args(["--rpc-url", &rpc]).assert_failure();
    assert_eq!(tip20.balanceOf(admin).call().await.unwrap(), U256::from(1000));
}

// T12 activates TIP-1006 `burnAt`, which only accounts holding `BURN_AT_ROLE` may call.
#[casttest]
async fn tip20_burn_at_role_enables_burn_at_on_t12(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T12.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let (admin, admin_pk) = dev_account(&handle, 0);
    let (burner, burner_pk) = dev_account(&handle, 1);
    let holder = handle.dev_accounts().nth(2).unwrap();
    let token = create_role_test_token(&mut cmd, &handle).await;
    let token_arg = token.to_string();
    let burner_arg = burner.to_string();
    let holder_arg = holder.to_string();
    let tip20 = ITIP20::new(token, &provider);
    let burn_at_role = tip20.BURN_AT_ROLE().call().await.unwrap();

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "issuer", &admin.to_string()])
        .args(["--private-key", &admin_pk, "--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args(["erc20", "mint", &token_arg, &holder_arg, "1000"])
        .args(["--private-key", &admin_pk, "--rpc-url", &rpc])
        .assert_success();

    let burn_at = ["send", &token_arg, "burnAt(address,uint256)", &holder_arg, "400"];
    cmd.cast_fuse()
        .args(burn_at)
        .args(["--private-key", &burner_pk, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Failed to estimate gas: server returned an error response: error code 3: execution reverted: custom error 0x82b42900, data: "0x82b42900": Unauthorized

"#]]);

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "burn-at", &burner_arg])
        .args(["--private-key", &admin_pk, "--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args([
            "--json",
            "tip20",
            "has-role",
            &token_arg,
            "burn-at",
            &burner_arg,
            "--rpc-url",
            &rpc,
        ])
        .assert_json_stdout(has_role_json(token, burn_at_role, Some("BURN_AT_ROLE"), burner, true));

    cmd.cast_fuse()
        .args(burn_at)
        .args(["--private-key", &burner_pk, "--rpc-url", &rpc])
        .assert_success();
    assert_eq!(tip20.balanceOf(holder).call().await.unwrap(), U256::from(600));
    assert_eq!(tip20.totalSupply().call().await.unwrap(), U256::from(600));
}

// Role updates are rejected before a transaction is sent when the sender does not hold the
// role's admin role, including an admin role reconfigured to a role the precompile does not name.
#[casttest]
async fn tip20_role_updates_require_role_admin(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T12.into()))).await;
    let rpc = handle.http_endpoint();
    let provider = handle.http_provider();
    let (admin, admin_pk) = dev_account(&handle, 0);
    let (delegate, delegate_pk) = dev_account(&handle, 1);
    let token = create_role_test_token(&mut cmd, &handle).await;
    let token_arg = token.to_string();
    let admin_arg = admin.to_string();
    let delegate_arg = delegate.to_string();
    let issuer_role = keccak256("ISSUER_ROLE");
    let issuer_admin_role = B256::with_last_byte(0xab);
    let issuer_admin_role_arg = issuer_admin_role.to_string();

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "issuer", &delegate_arg])
        .args(["--private-key", &delegate_pk, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 cannot grant ISSUER_ROLE on TIP-20 token 0x20c000000000000000000000A3C1274aaDd82e4D: it does not hold DEFAULT_ADMIN_ROLE, the role's admin role

"#]]);
    cmd.cast_fuse()
        .args(["tip20", "revoke-role", &token_arg, "admin", &admin_arg])
        .args(["--private-key", &delegate_pk, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: 0x70997970C51812dc3A010C7d01b50e0d17dc79C8 cannot revoke DEFAULT_ADMIN_ROLE on TIP-20 token 0x20c000000000000000000000A3C1274aaDd82e4D: it does not hold DEFAULT_ADMIN_ROLE, the role's admin role

"#]]);
    assert_eq!(provider.get_transaction_count(delegate).await.unwrap(), 0);

    let set_role_admin = TransactionRequest::default()
        .from(admin)
        .to(token)
        .with_input(
            IRolesAuth::new(token, &provider)
                .setRoleAdmin(issuer_role, issuer_admin_role)
                .calldata()
                .clone(),
        )
        .with_gas_limit(10_000_000);
    let receipt = provider
        .send_transaction(WithOtherFields::new(set_role_admin))
        .await
        .unwrap()
        .get_receipt()
        .await
        .unwrap();
    assert!(receipt.status(), "setRoleAdmin should succeed");

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "issuer", &admin_arg])
        .args(["--private-key", &admin_pk, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 cannot grant ISSUER_ROLE on TIP-20 token 0x20c000000000000000000000A3C1274aaDd82e4D: it does not hold 0x00000000000000000000000000000000000000000000000000000000000000ab, the role's admin role

"#]]);

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, &issuer_admin_role_arg, &delegate_arg])
        .args(["--private-key", &admin_pk, "--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args(["tip20", "has-role", &token_arg, &issuer_admin_role_arg, &delegate_arg])
        .args(["--rpc-url", &rpc])
        .assert_success()
        .stdout_eq(str![[r#"
Token:    0x20c000000000000000000000A3C1274aaDd82e4D
Role:     0x00000000000000000000000000000000000000000000000000000000000000ab
Account:  0x70997970C51812dc3A010C7d01b50e0d17dc79C8
Has role: true

"#]]);

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "issuer", &admin_arg])
        .args(["--private-key", &delegate_pk, "--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args(["--json", "tip20", "has-role", &token_arg, "issuer", &admin_arg, "--rpc-url", &rpc])
        .assert_json_stdout(has_role_json(token, issuer_role, Some("ISSUER_ROLE"), admin, true));
}

// Roles are plain hashes, so `BURN_AT_ROLE` can be granted ahead of T12 even though the `burnAt`
// selector it guards is not active yet.
#[casttest]
async fn tip20_burn_at_role_can_be_granted_before_t12(cmd: _) {
    let (_, handle) =
        anvil::spawn(NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T11.into()))).await;
    let rpc = handle.http_endpoint();
    let (_, admin_pk) = dev_account(&handle, 0);
    let (burner, burner_pk) = dev_account(&handle, 1);
    let holder = handle.dev_accounts().nth(2).unwrap();
    let token = create_role_test_token(&mut cmd, &handle).await;
    let token_arg = token.to_string();
    let burner_arg = burner.to_string();
    let burn_at_role = keccak256("BURN_AT_ROLE");

    cmd.cast_fuse()
        .args(["tip20", "grant-role", &token_arg, "burn-at", &burner_arg])
        .args(["--private-key", &admin_pk, "--rpc-url", &rpc])
        .assert_success();
    cmd.cast_fuse()
        .args([
            "--json",
            "tip20",
            "has-role",
            &token_arg,
            "burn-at",
            &burner_arg,
            "--rpc-url",
            &rpc,
        ])
        .assert_json_stdout(has_role_json(token, burn_at_role, Some("BURN_AT_ROLE"), burner, true));

    cmd.cast_fuse()
        .args(["send", &token_arg, "burnAt(address,uint256)", &holder.to_string(), "1"])
        .args(["--private-key", &burner_pk, "--rpc-url", &rpc])
        .assert_failure()
        .stderr_eq(str![[r#"
Error: Failed to estimate gas: server returned an error response: error code 3: execution reverted: custom error 0xaa4bc69a: 9803f21600000000000000000000000000000000000000000000000000000000, data: "0xaa4bc69a9803f21600000000000000000000000000000000000000000000000000000000": UnknownFunctionSelector(0x9803f216)

"#]]);
}

#[casttest]
fn tip20_role_commands_reject_unknown_role_names(cmd: _) {
    cmd.cast_fuse()
        .args(["tip20", "has-role", &PATH_USD_ADDRESS.to_string(), "minter"])
        .arg("0x0000000000000000000000000000000000000001")
        .assert_failure()
        .stderr_eq(str![[r#"
error: invalid value 'minter' for '<ROLE>': unknown TIP-20 role `minter`; expected one of admin, issuer, pause, unpause, burn-blocked, burn-at, or a 32-byte role hash

For more information, try '--help'.

"#]]);
}
