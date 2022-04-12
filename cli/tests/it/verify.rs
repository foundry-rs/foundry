//! Contains various tests for checking forge commands related to verifying contracts on etherscan

use ethers::types::Chain;
use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
};

fn etherscan_key() -> Option<String> {
    std::env::var("ETHERSCAN_API_KEY").ok()
}

fn network_rpc_key(chain: &str) -> Option<String> {
    let key = format!("{}_RPC_URL", chain.to_uppercase());
    std::env::var(&key).ok()
}

fn network_private_key(chain: &str) -> Option<String> {
    let key = format!("{}_PRIVATE_KEY", chain.to_uppercase());
    std::env::var(&key).ok()
}

/// Represents external input required for executing verification requests
struct VerifyExternalities {
    chain: Chain,
    rpc: String,
    pk: String,
    etherscan: String
}

impl VerifyExternalities {

    fn goerli() -> Option<Self> {
        Some(Self {
            chain: Chain::Goerli,
            rpc: network_rpc_key("goerli")?,
            pk: network_private_key("goerli")?,
            etherscan: etherscan_key()?
        })
    }

    fn commands(&self) -> Vec<String> {
        vec![
            "--chain".to_string(),
            self.chain.to_string(),
            "--rpc-url",
            self.rpc.clone(),
            "--private-key".to_string(),
            self.pk.clone(),
        ]
    }
}


/// Returns the current millis since unix epoch.
///
/// This way we generate unique contracts so, etherscan will always have to verify them
fn millis_since_epoch() -> u128 {
    let now = std::time::SystemTime::now();
    now.duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
        .as_millis()
}

/// Adds a `Unique` contract to the source directory of the project that can be imported as
/// `import {Unique} from "./unique.sol";`
fn add_unique(prj: &TestProject) {
    let timestamp = millis_since_epoch();
    prj.inner().add_source(
        "unique",
        format!(
            r#"
// SPDX-License-Identifier: UNLICENSED
pragma solidity >=0.4.0;

contract Unique {{
    uint public _timpestamp = {};
}}
"#,
            timestamp
        ),
    ).unwrap();
}

// tests that direct import paths are handled correctly
forgetest!(can_verify_random_contract, |prj: TestProject, mut cmd: TestCommand| {
    // only execute if keys present
    if let Some(info) = VerifyExternalities::goerli() {
        let VerifyExternalities{rpc, pk, etherscan} = info;

        add_unique(&prj);

        prj.inner()
            .add_source(
                "Verify.sol",
                r#"
    // SPDX-License-Identifier: UNLICENSED
    pragma solidity 0.8.10;
    import {Unique} from "./unique.sol";
    contract ToVerify is Unique {
        function doStuff() external {}
    }
       "#,
            )
            .unwrap();

        cmd.arg("build");
        cmd.print_output();

    }

});
