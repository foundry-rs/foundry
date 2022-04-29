//! Contains various tests for checking forge commands related to verifying contracts on etherscan

use crate::utils::{self, EnvExternalities};

use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
    Retry,
};
use std::time::Duration;

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
    uint public _timestamp = {};
}}
"#,
                timestamp
            ),
        )
        .unwrap();
}

fn verify_on_chain(info: Option<EnvExternalities>, prj: TestProject, mut cmd: TestCommand) {
    // only execute if keys present
    if let Some(info) = info {
        add_unique(&prj);

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

        let contract_path = "src/Verify.sol:Verify";

        cmd.arg("create");
        cmd.args(info.create_args()).arg(contract_path);

        let out = cmd.stdout_lossy();
        let address = utils::parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {out}"));

        cmd.forge_fuse().arg("verify-contract").root_arg().args([
            "--chain-id".to_string(),
            info.chain.to_string(),
            "--compiler-version".to_string(),
            "v0.8.10+commit.fc410830".to_string(),
            "--optimizer-runs".to_string(),
            "200".to_string(),
            address,
            contract_path.to_string(),
            info.etherscan.to_string(),
        ]);

        // `verify-contract`
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
        {
            cmd.forge_fuse()
                .arg("verify-check")
                .arg("--chain-id")
                .arg(info.chain.to_string())
                .arg(guid)
                .arg(info.etherscan);

            // give etherscan some time to verify the contract
            let retry = Retry::new(6, Some(Duration::from_secs(30)));
            retry
                .run(|| -> eyre::Result<()> {
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
                .expect("Failed to verify check")
        }
    }
}

// tests verify on goerli if correct env vars are set
forgetest!(can_verify_random_contract_goerli, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::goerli(), prj, cmd);
});

// tests verify on Fantom testnet if correct env vars are set
forgetest!(can_verify_random_contract_fantom_testnet, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::ftm_testnet(), prj, cmd);
});
