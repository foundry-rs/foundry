//! Contains various tests for checking forge commands related to verifying contracts on etherscan
//! and sourcify

use crate::utils::{self, EnvExternalities};
use foundry_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};
use foundry_utils::Retry;

/// Adds a `Unique` contract to the source directory of the project that can be imported as
/// `import {Unique} from "./unique.sol";`
fn add_unique(prj: &TestProject) {
    let timestamp = utils::millis_since_epoch();
    prj.inner()
        .add_source(
            "unique",
            format!(
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
    prj.inner()
        .add_source(
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

// tests `create && contract-verify && verify-check` on Fantom testnet if correct env vars are set
forgetest!(can_verify_random_contract_fantom_testnet, |prj, cmd| {
    verify_on_chain(EnvExternalities::ftm_testnet(), prj, cmd);
});

// tests `create && contract-verify && verify-check` on Optimism kovan if correct env vars are set
forgetest!(can_verify_random_contract_optimism_kovan, |prj, cmd| {
    verify_on_chain(EnvExternalities::optimism_kovan(), prj, cmd);
});
