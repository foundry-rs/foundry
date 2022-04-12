//! Contains various tests for checking forge commands related to verifying contracts on etherscan

use ethers::types::Chain;
use foundry_cli_test_utils::{
    forgetest,
    util::{TestCommand, TestProject},
    Retry,
};
use std::time::Duration;

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
    etherscan: String,
}

impl VerifyExternalities {
    fn goerli() -> Option<Self> {
        Some(Self {
            chain: Chain::Goerli,
            rpc: network_rpc_key("goerli")?,
            pk: network_private_key("goerli")?,
            etherscan: etherscan_key()?,
        })
    }

    /// Returns the arguments required to deploy the contract
    fn create_args(&self) -> Vec<String> {
        vec![
            "--chain".to_string(),
            self.chain.to_string(),
            "--rpc-url".to_string(),
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

// tests that direct import paths are handled correctly
forgetest!(can_verify_random_contract, |prj: TestProject, mut cmd: TestCommand| {
    // only execute if keys present
    if let Some(info) = VerifyExternalities::goerli() {
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
        let address = parse_deployed_address(out.as_str())
            .unwrap_or_else(|| panic!("Failed to parse deployer {}", out));

        cmd.forge_fuse().arg("verify-contract").root_arg().args([
            "--chain-id".to_string(),
            info.chain.to_string(),
            "--compiler-version".to_string(),
            "v0.8.10+commit.fc410830".to_string(),
            "--optimizer-runs".to_string(),
            "200".to_string(),
            address.clone(),
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
                    parse_verification_guid(&out).ok_or_else(|| {
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
                        "Failed to get verfiication, stdout: {}, stderr: {}",
                        out,
                        String::from_utf8_lossy(&output.stderr)
                    )
                })
                .expect("Failed to verify check")
        }
    }
});

/// Parses the address the contract was deployed to
fn parse_deployed_address(out: &str) -> Option<String> {
    for line in out.lines() {
        if line.starts_with("Deployed to") {
            return Some(line.trim_start_matches("Deployed to: ").to_string())
        }
    }
    None
}

fn parse_verification_guid(out: &str) -> Option<String> {
    for line in out.lines() {
        if line.contains("GUID") {
            return Some(line.replace("GUID:", "").replace("`", "").trim().to_string())
        }
    }
    None
}
