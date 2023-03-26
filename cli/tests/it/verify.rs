//! Contains various tests for checking forge commands related to verifying contracts on etherscan
//! and sourcify

use crate::utils::{self, EnvExternalities};
use ethers::solc::artifacts::BytecodeHash;
use foundry_cli_test_utils::{
    forgetest, forgetest_async,
    util::{TestCommand, TestProject},
};
use foundry_config::Config;
use foundry_utils::Retry;
use std::{fs, path::PathBuf};

const VERIFICATION_PROVIDERS: &[&str] = &["etherscan", "sourcify"];

/// Adds a `Unique` contract to the source directory of the project that can be imported as
/// `import {Unique} from "./unique.sol";`
fn add_unique(prj: &TestProject) {
    let timestamp = utils::millis_since_epoch();
    prj.inner()
        .add_source(
            "unique",
            format!(
                r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.4.0;

contract Unique {{
    uint public _timestamp = {timestamp};
}}
"#
            ),
        )
        .unwrap();
}

fn add_verify_target(prj: &TestProject) {
    prj.inner()
        .add_source(
            "Verify.sol",
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity =0.8.10;
import {Unique} from "./unique.sol";
contract Verify is Unique {
function doStuff() external {}
}
"#,
        )
        .unwrap();
}

fn parse_verification_result(cmd: &mut TestCommand, retries: u32) -> eyre::Result<()> {
    // give etherscan some time to verify the contract
    let retry = Retry::new(retries, Some(30));
    retry.run(|| -> eyre::Result<()> {
        let output = cmd.unchecked_output();
        let out = String::from_utf8_lossy(&output.stdout);
        if out.contains("Contract successfully verified") {
            return Ok(())
        }
        eyre::bail!(
            "Failed to get verification, stdout: {}, stderr: {}",
            out,
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn verify_on_chain(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        println!("verifying on {}", info.chain);
        add_unique(&prj);
        add_verify_target(&prj);

        let contract_path = "src/Verify.sol:Verify";
        cmd.arg("create").args(info.create_args()).arg(contract_path);

        let out = cmd.stdout_lossy();
        let address = utils::parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {out}"));

        cmd.forge_fuse().arg("verify-contract").root_arg().args([
            "--chain-id".to_string(),
            info.chain.to_string(),
            address,
            contract_path.to_string(),
            info.etherscan.to_string(),
            "--verifier".to_string(),
            info.verifier.to_string(),
        ]);

        // `verify-contract`
        let guid = {
            // give etherscan some time to detect the transaction
            let retry = Retry::new(5, Some(60));
            retry
                .run(|| -> eyre::Result<String> {
                    let output = cmd.unchecked_output();
                    let out = String::from_utf8_lossy(&output.stdout);
                    utils::parse_verification_guid(&out).ok_or_else(|| {
                        eyre::eyre!(
                            "Failed to get guid, stdout: {}, stderr: {}",
                            out,
                            String::from_utf8_lossy(&output.stderr)
                        )
                    })
                })
                .expect("Failed to get verify guid")
        };

        // verify-check
        cmd.forge_fuse()
            .arg("verify-check")
            .arg(guid)
            .arg("--chain-id")
            .arg(info.chain.to_string())
            .arg("--etherscan-key")
            .arg(info.etherscan)
            .arg("--verifier")
            .arg(info.verifier);

        parse_verification_result(&mut cmd, 6).expect("Failed to verify check")
    }
}

fn verify_watch_on_chain(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        add_unique(&prj);
        add_verify_target(&prj);

        let contract_path = "src/Verify.sol:Verify";
        cmd.arg("create").args(info.create_args()).arg(contract_path);

        let out = cmd.stdout_lossy();
        let address = utils::parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {out}"));

        // `verify-contract`
        cmd.forge_fuse().arg("verify-contract").root_arg().args([
            "--chain-id".to_string(),
            info.chain.to_string(),
            "--watch".to_string(),
            address,
            contract_path.to_string(),
            info.etherscan,
        ]);
        parse_verification_result(&mut cmd, 6).expect("Failed to verify check")
    }
}

fn verify_flag_on_create_on_chain(
    info: Option<EnvExternalities>,
    prj: TestProject,
    mut cmd: TestCommand,
) {
    // only execute if keys present
    if let Some(info) = info {
        for verifier in VERIFICATION_PROVIDERS {
            println!("verifying with {verifier}");

            add_unique(&prj);
            add_verify_target(&prj);

            println!("root {:?}", prj.root());

            let contract_path = "src/Verify.sol:Verify";

            cmd.arg("create")
                .args(info.create_args())
                .arg("--verify")
                .arg(contract_path)
                .arg("--verifier")
                .arg(verifier);

            parse_verification_result(&mut cmd, 1).expect("Failed to verify check");

            // reset command
            cmd.forge_fuse();
        }
    }
}

// tests `create && contract-verify && verify-check` on goerli if correct env vars are set
forgetest!(can_verify_random_contract_goerli, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::goerli(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Fantom testnet if correct env vars are set
forgetest!(can_verify_random_contract_fantom_testnet, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::ftm_testnet(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Optimism kovan if correct env vars are set
forgetest!(can_verify_random_contract_optimism_kovan, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::optimism_kovan(), prj, cmd);
});

forgetest!(can_verify_random_contract_arbitrum_goerli, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::arbitrum_goerli(), prj, cmd);
});

// tests `create && contract-verify --watch` on goerli if correct env vars are set
forgetest!(can_verify_watch_random_contract_goerli, |prj: TestProject, cmd: TestCommand| {
    verify_watch_on_chain(EnvExternalities::goerli(), prj, cmd);
});

// tests `create --verify` on goerli if correct env vars are set
forgetest!(can_verify_on_create_random_contract_goerli, |prj: TestProject, cmd: TestCommand| {
    verify_flag_on_create_on_chain(EnvExternalities::goerli(), prj, cmd);
});

// tests `create --verify` on goerli with constructor args
forgetest!(
    can_verify_on_create_random_contract_constructor_args_goerli,
    |prj: TestProject, mut cmd: TestCommand| {
        if let Some(info) = EnvExternalities::goerli() {
            add_unique(&prj);
            prj.inner()
                .add_source(
                    "Verify.sol",
                    r#"
    // SPDX-License-Identifier: UNLICENSED
    pragma solidity =0.8.10;
    import {Unique} from "./unique.sol";
    contract Verify is Unique {
        address _a;
        address _b;
        address _c;
        address _d;
        constructor(address a, address b, address c, address d)  {
            _a = a;
            _b = b;
            _c = c;
            _d = d;
        }
    }
    "#,
                )
                .unwrap();

            for verifier in VERIFICATION_PROVIDERS {
                let contract_path = "src/Verify.sol:Verify";
                cmd.arg("create")
                    .args(info.create_args())
                    .args([
                        "--constructor-args",
                        "0x82A0F5F531F9ce0df1DF5619f74a0d3fA31FF561",
                        "0xE592427A0AEce92De3Edee1F18E0157C05861564",
                        "0x1717A0D5C8705EE89A8aD6E808268D6A826C97A4",
                        "0xc778417E063141139Fce010982780140Aa0cD5Ab",
                    ])
                    .arg("--verify")
                    .arg(contract_path)
                    .arg("--verifier")
                    .arg(verifier);

                parse_verification_result(&mut cmd, 1).expect("Failed to verify check")
            }
        }
    }
);

// tests `script --verify` on goerli with contract + predeployed libraries
forgetest!(
    can_verify_on_script_random_contract_with_libs_goerli,
    |prj: TestProject, mut cmd: TestCommand| {
        if let Some(info) = EnvExternalities::goerli() {
            add_unique(&prj);
            prj.inner()
                .add_source(
                    "ScriptVerify.sol",
                    r#"
    // SPDX-License-Identifier: UNLICENSED
    pragma solidity =0.8.10;
    import {Unique} from "./unique.sol";
    library F {
        function f() public pure returns (uint256) {
            return 1;
        }
    }
    library C {
        function c() public pure returns (uint256) {
            return 2;
        }
    }
    interface HEVM {
        function startBroadcast() external;
    }

    contract Hello is Unique {
        function world() public {
            F.f();
            C.c();
        }
    }
    contract ScriptVerify {
        function run() public {
            address vm = address(bytes20(uint160(uint256(keccak256('hevm cheat code')))));
            HEVM(vm).startBroadcast();
            new Hello();
        }
    }
    "#,
                )
                .unwrap();

            let contract_path = "src/ScriptVerify.sol:ScriptVerify";
            cmd.arg("script")
                .args(vec![
                    "--rpc-url".to_string(),
                    info.rpc.clone(),
                    "--private-key".to_string(),
                    info.pk,
                ])
                .arg("--broadcast")
                .arg("--verify")
                .arg(contract_path);

            parse_verification_result(&mut cmd, 1).expect("Failed to verify check")
        }
    }
);

// tests `script --verify` by deploying on goerli and verifying it on etherscan
// Uses predeployed libs and contract creations inside constructors and calls
forgetest_async!(
    test_live_can_deploy_and_verify,
    |prj: TestProject, mut cmd: TestCommand| async move {
        let info = EnvExternalities::goerli();

        // ignore if etherscan var not set
        if std::env::var("ETHERSCAN_API_KEY").is_err() {
            eprintln!("Goerli secrets not set.");
            return
        }

        let info = info.expect("Missing goerli env vars");
        println!("Verifying via {:?}", info.address());

        add_unique(&prj);

        prj.inner()
            .add_source(
                "ScriptVerify.sol",
                fs::read_to_string(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/ScriptVerify.sol"),
                )
                .unwrap(),
            )
            .unwrap();

        // explicitly byte code hash for consistent checks
        let config = Config { bytecode_hash: BytecodeHash::None, ..Default::default() };
        prj.write_config(config);

        let contract_path = "src/ScriptVerify.sol:ScriptVerify";
        cmd.args(vec![
            "script",
            "--rpc-url",
            &info.rpc,
            "--private-key",
            &info.pk,
            "--broadcast",
            "-vvvv",
            "--slow",
            "--optimize",
            "--verify",
            "--optimizer-runs",
            "200",
            "--use",
            "0.8.16",
            "--retries",
            "10",
            "--delay",
            "20",
            contract_path,
        ]);

        let output = cmd.unchecked_output();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let err = format!(
            "Failed to get verification, stdout: {}, stderr: {}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        );

        // ensure we're sending all 5 transactions
        assert!(stdout.contains("Sending transactions [0 - 4]"), "{}", err);

        // Note: the 5th tx creates contracts internally, which are little flaky at times because
        // the goerli etherscan indexer can take a long time to index these contracts

        // ensure transactions are successful
        assert!(stdout.matches('✅').count() >= 4, "{}", err);

        // ensure verified all deployments
        assert!(stdout.matches("Contract successfully verified").count() >= 4, "{}", err);
    }
);
