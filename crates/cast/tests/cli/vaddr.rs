//! CLI tests for vaddr commands.

use super::*;

// Tests for `cast vaddr` JSON output
casttest!(vaddr_create_json_output, |_prj, cmd| {
    // Use a pre-computed salt that satisfies the 4-byte PoW requirement for this owner.
    // Salt: 0x0000000000000000000000000000000000000000000000003ee0a78d00000000
    // Owner: 0x1234567890123456789012345678901234567890
    let out = cmd
        .args([
            "--json",
            "vaddr",
            "create",
            "--owner",
            "0x1234567890123456789012345678901234567890",
            "--salt",
            "0x0000000000000000000000000000000000000000000000003ee0a78d00000000",
            "--no-register",
            "--count",
            "2",
        ])
        .assert_success()
        .get_output()
        .stdout_lossy();

    let envelope: serde_json::Value = serde_json::from_str(out.trim()).expect("valid JSON");
    let v = &envelope["data"];
    assert_eq!(v["salt"], "0x0000000000000000000000000000000000000000000000003ee0a78d00000000");
    assert_eq!(
        v["registration_hash"],
        "0x000000002f51c0c4f66f3910f799c6b98e2123ef43a401a062eb8ee07498c396"
    );
    assert_eq!(v["master_id"], "0x2f51c0c4");
    let addrs = v["virtual_addresses"].as_array().expect("array");
    assert_eq!(addrs.len(), 2);
    assert_eq!(addrs[0]["tag"], "0x000000000000");
    assert_eq!(
        addrs[0]["address"].as_str().unwrap().to_lowercase(),
        "0x2f51c0c4fdfdfdfdfdfdfdfdfdfd000000000000"
    );
    assert_eq!(addrs[1]["tag"], "0x000000000001");
    assert_eq!(
        addrs[1]["address"].as_str().unwrap().to_lowercase(),
        "0x2f51c0c4fdfdfdfdfdfdfdfdfdfd000000000001"
    );
});

casttest!(vaddr_create_plain_output, |_prj, cmd| {
    cmd.args([
        "vaddr",
        "create",
        "--owner",
        "0x1234567890123456789012345678901234567890",
        "--salt",
        "0x0000000000000000000000000000000000000000000000003ee0a78d00000000",
        "--no-register",
    ])
    .assert_success()
    .stdout_eq(str![[r#"
Salt:              0x0000000000000000000000000000000000000000000000003ee0a78d00000000
Registration hash: 0x000000002f51c0c4f66f3910f799c6b98e2123ef43a401a062eb8ee07498c396
Master ID:         0x2f51c0c4

Virtual addresses:
  tag=0x000000000000  [..]

"#]]);
});

// End-to-end `cast vaddr` tests against a local Anvil Tempo node.
//
// These tests exercise the full TIP-1022 lifecycle, including mining a 4-byte PoW salt.
// Keep them out of the default local suite because the mining step is intentionally CPU-bound
// and can saturate developer machines. Run explicitly with `--ignored` when changing this flow.

mod vaddr_e2e {
    use super::*;
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
        time::{Duration, Instant},
    };
    use tempo_contracts::precompiles::DEFAULT_FEE_TOKEN;
    use tempo_hardfork::TempoHardfork;

    /// `cast vaddr` exercises TIP-1022, which is enabled after `TempoHardfork::T3`.
    fn tempo_t3_config() -> anvil::NodeConfig {
        anvil::NodeConfig::test_tempo().with_hardfork(Some(TempoHardfork::T3.into()))
    }

    /// Number of mining threads — use all available CPUs to keep wall time
    /// down on hosts where nextest limits per-test parallelism.
    fn mining_threads() -> String {
        std::thread::available_parallelism().map_or(8, |n| n.get()).to_string()
    }

    /// Run `cast vaddr create` (mine + register) and parse the user-tag-zero virtual
    /// address from the plain-text output.
    fn create_and_register_vaddr(
        cmd: &mut foundry_test_utils::TestCommand,
        rpc: &str,
        owner: &PrivateKeySigner,
    ) -> String {
        let owner_pk = format!("0x{}", hex::encode(owner.credential().to_bytes()));
        let owner_addr = format!("{:#x}", owner.address());
        let out = cmd
            .cast_fuse()
            .args([
                "vaddr",
                "create",
                "--owner",
                &owner_addr,
                "--private-key",
                &owner_pk,
                "-j",
                &mining_threads(),
                "--rpc-url",
                rpc,
            ])
            .assert_success()
            .get_output()
            .stdout_lossy();

        out.lines()
            .find_map(|line| {
                let rest = line.trim_start().strip_prefix("tag=0x000000000000")?;
                rest.split_whitespace().next().map(str::to_string)
            })
            .unwrap_or_else(|| panic!("could not parse vaddr from create output:\n{out}"))
    }

    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_create_register_json_includes_tx_hash,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();
            let owner_pk = format!("0x{}", hex::encode(owner.credential().to_bytes()));
            let owner_addr = format!("{:#x}", owner.address());

            let out = cmd
                .cast_fuse()
                .args([
                    "--json",
                    "vaddr",
                    "create",
                    "--owner",
                    &owner_addr,
                    "--private-key",
                    &owner_pk,
                    "-j",
                    &mining_threads(),
                    "--rpc-url",
                    &rpc,
                ])
                .assert_success()
                .get_output()
                .stdout_lossy();

            let envelope: serde_json::Value =
                serde_json::from_str(out.trim()).expect("create --json output is valid JSON");
            let tx_hash = envelope["data"]["registration_tx_hash"]
                .as_str()
                .expect("registration_tx_hash is a string");
            B256::from_str(tx_hash).expect("registration_tx_hash is a valid tx hash");
        }
    );

    // `cast vaddr create` mines a PoW salt, registers a virtual master on-chain,
    // and `cast vaddr resolve` returns the registered owner.
    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_create_register_and_resolve,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();

            let vaddr = create_and_register_vaddr(&mut cmd, &rpc, &owner);

            let resolve_out = cmd
                .cast_fuse()
                .args(["--json", "vaddr", "resolve", &vaddr, "--rpc-url", &rpc])
                .assert_success()
                .get_output()
                .stdout_lossy();

            let v: serde_json::Value = serde_json::from_str(resolve_out.trim())
                .expect("resolve --json output is valid JSON");
            assert_eq!(
                v["address"].as_str().unwrap().to_lowercase(),
                vaddr.to_lowercase(),
                "resolve.address mismatch: {resolve_out}"
            );
            assert_eq!(
                v["master_address"].as_str().unwrap().to_lowercase(),
                format!("{:#x}", owner.address()),
                "resolve.master_address should match the registered owner: {resolve_out}"
            );
        }
    );

    // Transferring a TIP-20 fee token to a registered virtual address must
    // auto-forward the deposit to the master wallet at the protocol level.
    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_auto_forward_to_master,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();
            let sender = handle.dev_wallets().nth(1).unwrap();
            let sender_pk = format!("0x{}", hex::encode(sender.credential().to_bytes()));
            let owner_addr = format!("{:#x}", owner.address());

            let vaddr = create_and_register_vaddr(&mut cmd, &rpc, &owner);

            let balance = |cmd: &mut foundry_test_utils::TestCommand| -> u128 {
                let out = cmd
                    .cast_fuse()
                    .args([
                        "call",
                        &DEFAULT_FEE_TOKEN.to_string(),
                        "balanceOf(address)(uint256)",
                        &owner_addr,
                        "--rpc-url",
                        &rpc,
                    ])
                    .assert_success()
                    .get_output()
                    .stdout_lossy();
                // `cast call` with a typed signature prints e.g. `1000000000000 [1e12]`.
                out.split_whitespace().next().unwrap().parse().unwrap()
            };

            let before = balance(&mut cmd);

            let amount: u128 = 1_000_000;
            cmd.cast_fuse()
                .args([
                    "send",
                    &DEFAULT_FEE_TOKEN.to_string(),
                    "transfer(address,uint256)",
                    &vaddr,
                    &amount.to_string(),
                    "--rpc-url",
                    &rpc,
                    "--private-key",
                    &sender_pk,
                ])
                .assert_success();

            let after = balance(&mut cmd);
            assert_eq!(
                after - before,
                amount,
                "transfer to virtual address should auto-forward to master (before={before}, after={after})"
            );
        }
    );

    // `cast vaddr watch --from-block` must replay historical TIP-20 Transfer
    // logs targeted at the virtual address. The command then polls forever, so
    // we spawn it as a child process and kill it once we observe the expected
    // historical line (or the deadline elapses).
    casttest!(
        #[ignore = "mines a TIP-1022 salt and saturates local CPUs"]
        vaddr_watch_historical,
        async |_prj, cmd| {
            let (_api, handle) = anvil::spawn(tempo_t3_config()).await;
            let rpc = handle.http_endpoint();
            let owner = handle.dev_wallets().next().unwrap();
            let sender = handle.dev_wallets().nth(1).unwrap();
            let sender_pk = format!("0x{}", hex::encode(sender.credential().to_bytes()));
            let sender_addr = format!("{:#x}", sender.address());

            let vaddr = create_and_register_vaddr(&mut cmd, &rpc, &owner);

            // Capture the block before the transfer so `--from-block` replays it.
            let block_before_out = cmd
                .cast_fuse()
                .args(["block-number", "--rpc-url", &rpc])
                .assert_success()
                .get_output()
                .stdout_lossy();
            let block_before: u64 = block_before_out.trim().parse().unwrap();

            let amount: u128 = 1_000_000;
            cmd.cast_fuse()
                .args([
                    "send",
                    &DEFAULT_FEE_TOKEN.to_string(),
                    "transfer(address,uint256)",
                    &vaddr,
                    &amount.to_string(),
                    "--rpc-url",
                    &rpc,
                    "--private-key",
                    &sender_pk,
                ])
                .assert_success();

            // Spawn `cast vaddr watch` as a child process (it loops indefinitely).
            cmd.cast_fuse().args([
                "vaddr",
                "watch",
                &vaddr,
                "--token",
                &DEFAULT_FEE_TOKEN.to_string(),
                "--from-block",
                &block_before.to_string(),
                "--rpc-url",
                &rpc,
            ]);
            let mut child =
                cmd.cmd().stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();

            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let expected = format!(
                "token={} from={} amount={}",
                DEFAULT_FEE_TOKEN.to_string().to_lowercase(),
                sender_addr,
                amount
            );

            let deadline = Instant::now() + Duration::from_secs(15);
            let mut captured = String::new();
            let mut found = false;
            while Instant::now() < deadline {
                let mut line = String::new();
                match stdout.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        captured.push_str(&line);
                        if line.to_lowercase().contains(&expected) {
                            found = true;
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = child.kill();
            let _ = child.wait();

            assert!(
                found,
                "cast vaddr watch did not emit historical transfer; expected substring `{expected}` in:\n{captured}"
            );
        }
    );
}
