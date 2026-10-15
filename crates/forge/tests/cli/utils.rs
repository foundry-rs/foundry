//! Various helper functions

use alloy_chains::NamedChain;
use std::{
    io::Read,
    path::Path,
    process::{Child, Command, Output, Stdio},
    thread::{self, JoinHandle},
};

/// Returns the current millis since unix epoch.
///
/// This way we generate unique contracts so, etherscan will always have to verify them
pub fn millis_since_epoch() -> u128 {
    let now = std::time::SystemTime::now();
    now.duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {now:?} is invalid: {err:?}"))
        .as_millis()
}

pub fn etherscan_key() -> Option<String> {
    std::env::var("ETHERSCAN_API_KEY").ok()
}

pub fn network_rpc_key(chain: &str) -> Option<String> {
    let key = format!("{}_RPC_URL", chain.to_uppercase().replace('-', "_"));
    std::env::var(key).ok()
}

/// Resolves the deployer key for `chain`, most specific first:
///
/// 1. `<NETWORK>_PRIVATE_KEY`, to point one network at its own account.
/// 2. `TESTNET_DEPLOYER_PRIVATE_KEY`, the shared throwaway deployer these tests fund.
/// 3. `TEST_PRIVATE_KEY`, kept for existing local setups.
///
/// Prefer the dedicated name over `TEST_PRIVATE_KEY`: it is generic enough that an unrelated value
/// left in the environment would otherwise deploy from an account the caller did not intend.
pub fn network_private_key(chain: &str) -> Option<String> {
    let key = format!("{}_PRIVATE_KEY", chain.to_uppercase().replace('-', "_"));
    std::env::var(key)
        .or_else(|_| std::env::var("TESTNET_DEPLOYER_PRIVATE_KEY"))
        .or_else(|_| std::env::var("TEST_PRIVATE_KEY"))
        .ok()
}

/// Represents external input required for executing verification requests
pub struct EnvExternalities {
    pub chain: NamedChain,
    pub rpc: String,
    pub pk: String,
    pub etherscan: String,
    pub verifier: String,
    pub verifier_url: Option<String>,
}

impl EnvExternalities {
    /// Externalities for a deploy + verify run of `chain` against `verifier`.
    ///
    /// `network` is the name used to look up `<NETWORK>_RPC_URL` and `<NETWORK>_PRIVATE_KEY`, and
    /// can differ from the canonical `NamedChain::as_str` spelling. Blockscout instances have no
    /// shared registry, so they must be given an explicit `verifier_url`.
    ///
    /// Returns `None` when the network is not configured, which is how these tests stay inert
    /// outside of the nightly workflow that supplies the funded deployer key.
    pub fn deploy_verify(
        chain: NamedChain,
        network: &str,
        verifier: &str,
        verifier_url: Option<&str>,
    ) -> Option<Self> {
        Some(Self {
            chain,
            rpc: network_rpc_key(network)?,
            pk: network_private_key(network)?,
            // Only Etherscan authenticates; Sourcify and Blockscout take no key.
            etherscan: if verifier == "etherscan" { etherscan_key()? } else { String::new() },
            verifier: verifier.to_string(),
            verifier_url: verifier_url.map(str::to_string),
        })
    }

    /// Returns the arguments required to deploy the contract
    pub fn create_args(&self) -> Vec<String> {
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

/// Parses the address the contract was deployed to
pub fn parse_deployed_address(out: &str) -> Option<String> {
    for line in out.lines() {
        if line.starts_with("Deployed to") {
            return Some(line.trim_start_matches("Deployed to: ").to_string());
        }
    }
    None
}

pub fn assert_debug_dump_identifies_contract(dump_path: &Path, address: &str, contract_name: &str) {
    let dump: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dump_path).unwrap()).unwrap();
    let identified = dump["contracts"]["identified_contracts"].as_object().unwrap();
    let target_identified = identified.iter().any(|(identified_address, name)| {
        identified_address.eq_ignore_ascii_case(address)
            && name.as_str().is_some_and(|name| name == contract_name)
    });
    assert!(target_identified, "forked target was not identified in debugger dump: {identified:?}");
}

/// Generates a string containing the code of a Solidity contract.
///
/// This contract compiles to a large init bytecode size, but small runtime size.
pub fn generate_large_init_contract(n: usize) -> String {
    let data = vec![0xff; n];
    let hex = alloy_primitives::hex::encode(data);
    format!(
        "\
contract LargeContract {{
    constructor() {{
        bytes memory data = hex\"{hex}\";
        assembly {{
            pop(mload(data))
        }}
    }}
}}    
"
    )
}

/// Generates a Solidity contract with both runtime and initcode bytecode at
/// least `n` bytes long, by embedding an `n`-byte hex constant returned from
/// an external `pure` function.
pub fn generate_large_runtime_contract(n: usize) -> String {
    let data = vec![0xff; n];
    let hex = alloy_primitives::hex::encode(data);
    format!(
        "\
contract LargeRuntime {{
    function data() external pure returns (bytes memory) {{
        return hex\"{hex}\";
    }}
}}
"
    )
}

/// A spawned child process that is killed when dropped.
pub struct KillOnDrop {
    child: Option<Child>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl KillOnDrop {
    pub fn spawn(command: &mut Command) -> Self {
        let mut child = command.stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
        let mut child_stderr = child.stderr.take().unwrap();
        let stderr = thread::spawn(move || {
            let mut stderr = Vec::new();
            child_stderr.read_to_end(&mut stderr).unwrap();
            stderr
        });
        Self { child: Some(child), stderr: Some(stderr) }
    }

    pub fn is_running(&mut self) -> bool {
        self.child.as_mut().unwrap().try_wait().unwrap().is_none()
    }

    pub fn kill_and_wait(mut self) -> Output {
        let mut child = self.child.take().unwrap();
        child.kill().unwrap();
        let status = child.wait().unwrap();
        Output { status, stdout: Vec::new(), stderr: self.stderr.take().unwrap().join().unwrap() }
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(stderr) = self.stderr.take() {
            let _ = stderr.join();
        }
    }
}
