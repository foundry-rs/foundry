//! Contains various tests for checking forge commands related to verifying contracts on Etherscan
//! and Sourcify.

use crate::utils::{self, EnvExternalities};
use foundry_common::retry::Retry;
use foundry_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};
use std::time::Duration;

/// Adds a `Unique` contract to the source directory of the project that can be imported as
/// `import {Unique} from "./unique.sol";`
fn add_unique(prj: &TestProject) {
    let timestamp = utils::millis_since_epoch();
    prj.add_source(
        "unique",
        &format!(
            r#"
contract Unique {{
    uint public _timestamp = {timestamp};
}}
"#
        ),
    )
    .unwrap();
}

fn add_verify_target(prj: &TestProject) {
    prj.add_source(
        "Verify.sol",
        r#"
import {Unique} from "./unique.sol";
contract Verify is Unique {
function doStuff() external {}
}
"#,
    )
    .unwrap();
}

fn add_single_verify_target_file(prj: &TestProject) {
    let timestamp = utils::millis_since_epoch();
    let contract = format!(
        r#"
contract Unique {{
    uint public _timestamp = {timestamp};
}}
contract Verify is Unique {{
function doStuff() external {{}}
}}
"#
    );

    prj.add_source("Verify.sol", &contract).unwrap();
}

fn add_verify_target_with_constructor(prj: &TestProject) {
    prj.add_source(
        "Verify.sol",
        r#"
import {Unique} from "./unique.sol";
contract Verify is Unique {
    struct SomeStruct {
        uint256 a;
        string str;
    }

    constructor(SomeStruct memory st, address owner) {}
}
"#,
    )
    .unwrap();
}

fn parse_verification_result(cmd: &mut TestCommand, retries: u32) -> eyre::Result<()> {
    // give etherscan some time to verify the contract
    let retry = Retry::new(retries, Some(Duration::from_secs(30)));
    retry.run(|| -> eyre::Result<()> {
        let output = cmd.unchecked_output();
        let out = String::from_utf8_lossy(&output.stdout);
        println!("{out}");
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

fn await_verification_response(info: EnvExternalities, mut cmd: TestCommand) {
    let guid = {
        // give etherscan some time to detect the transaction
        let retry = Retry::new(5, Some(Duration::from_secs(60)));
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
        .arg("--etherscan-api-key")
        .arg(info.etherscan)
        .arg("--verifier")
        .arg(info.verifier);

    parse_verification_result(&mut cmd, 6).expect("Failed to verify check")
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
            "--etherscan-api-key".to_string(),
            info.etherscan.to_string(),
            "--verifier".to_string(),
            info.verifier.to_string(),
        ]);

        await_verification_response(info, cmd)
    }
}

fn guess_constructor_args(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        println!("verifying on {}", info.chain);
        add_unique(&prj);
        add_verify_target_with_constructor(&prj);

        let contract_path = "src/Verify.sol:Verify";
        cmd.arg("create").args(info.create_args()).arg(contract_path).args(vec![
            "--constructor-args",
            "(239,SomeString)",
            "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045",
        ]);

        let out = cmd.stdout_lossy();
        let address = utils::parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {out}"));

        cmd.forge_fuse().arg("verify-contract").root_arg().args([
            "--rpc-url".to_string(),
            info.rpc.to_string(),
            address,
            contract_path.to_string(),
            "--etherscan-api-key".to_string(),
            info.etherscan.to_string(),
            "--verifier".to_string(),
            info.verifier.to_string(),
            "--guess-constructor-args".to_string(),
        ]);

        await_verification_response(info, cmd)
    }
}

/// Executes create --verify on the given chain
fn create_verify_on_chain(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        println!("verifying on {}", info.chain);
        add_single_verify_target_file(&prj);

        let contract_path = "src/Verify.sol:Verify";
        cmd.arg("create").args(info.create_args()).args([
            contract_path,
            "--etherscan-api-key",
            info.etherscan.as_str(),
            "--verify",
        ]);

        let out = cmd.stdout_lossy();
        assert!(out.contains("Contract successfully verified"), "{}", out);
    }
}

// tests `create && contract-verify && verify-check` on Fantom testnet if correct env vars are set
forgetest!(can_verify_random_contract_fantom_testnet, |prj, cmd| {
    verify_on_chain(EnvExternalities::ftm_testnet(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Optimism kovan if correct env vars are set
forgetest!(can_verify_random_contract_optimism_kovan, |prj, cmd| {
    verify_on_chain(EnvExternalities::optimism_kovan(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Sepolia testnet if correct env vars are set
forgetest!(can_verify_random_contract_sepolia, |prj, cmd| {
    verify_on_chain(EnvExternalities::sepolia(), prj, cmd);
});

// tests `create --verify on Sepolia testnet if correct env vars are set
// SEPOLIA_RPC_URL=https://rpc.sepolia.org
// TEST_PRIVATE_KEY=0x...
// ETHERSCAN_API_KEY=
forgetest!(can_create_verify_random_contract_sepolia, |prj, cmd| {
    create_verify_on_chain(EnvExternalities::sepolia(), prj, cmd);
});

// tests `create && contract-verify --guess-constructor-args && verify-check` on Goerli testnet if
// correct env vars are set
forgetest!(can_guess_constructor_args, |prj, cmd| {
    guess_constructor_args(EnvExternalities::goerli(), prj, cmd);
});
