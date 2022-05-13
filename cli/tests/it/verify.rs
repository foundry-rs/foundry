//! Contains various tests for checking forge commands related to verifying contracts on etherscan

use crate::utils::{self, EnvExternalities};

use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};

/// Adds a `Unique` contract to the source directory of the project that can be imported as
/// `import {Unique} from "./unique.sol";`
pub fn add_unique(prj: &TestProject) {
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
        cmd.args(info.create_args()).arg("--verify").arg(contract_path);

        let output = cmd.unchecked_output();
        let out = String::from_utf8_lossy(&output.stdout);
        utils::parse_verification_guid(&out).expect(&format!(
            "Failed to get guid, stdout: {}, stderr: {}",
            out,
            String::from_utf8_lossy(&output.stderr)
        ));
        if !out.contains("Contract successfully verified") {
            panic!(
                "Failed to get verification, stdout: {}, stderr: {}",
                out,
                String::from_utf8_lossy(&output.stderr)
            );
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

// tests verify on Optimism kovan if correct env vars are set
forgetest!(can_verify_random_contract_optimism_kovan, |prj: TestProject, cmd: TestCommand| {
    verify_on_chain(EnvExternalities::optimism_kovan(), prj, cmd);
});
