//! Various helper functions

use alloy_chains::NamedChain;
use alloy_primitives::Address;
use alloy_signer_local::PrivateKeySigner;
use std::path::Path;

/// Returns the current millis since unix epoch.
///
/// This way we generate unique contracts so, etherscan will always have to verify them
pub fn millis_since_epoch() -> u128 {
    let now = std::time::SystemTime::now();
    now.duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {now:?} is invalid: {err:?}"))
        .as_millis()
}

pub fn etherscan_key(chain: NamedChain) -> Option<String> {
    match chain {
        NamedChain::Fantom | NamedChain::FantomTestnet => {
            std::env::var("FTMSCAN_API_KEY").or_else(|_| std::env::var("FANTOMSCAN_API_KEY")).ok()
        }
        NamedChain::OptimismKovan => std::env::var("OP_KOVAN_API_KEY").ok(),
        _ => std::env::var("ETHERSCAN_API_KEY").ok(),
    }
}

pub fn network_rpc_key(chain: &str) -> Option<String> {
    let key = format!("{}_RPC_URL", chain.to_uppercase().replace('-', "_"));
    std::env::var(key).ok()
}

pub fn network_private_key(chain: &str) -> Option<String> {
    let key = format!("{}_PRIVATE_KEY", chain.to_uppercase().replace('-', "_"));
    std::env::var(key).or_else(|_| std::env::var("TEST_PRIVATE_KEY")).ok()
}

/// Represents external input required for executing verification requests
pub struct EnvExternalities {
    pub chain: NamedChain,
    pub rpc: String,
    pub pk: String,
    pub etherscan: String,
    pub verifier: String,
}

impl EnvExternalities {
    pub fn address(&self) -> Option<Address> {
        let pk: PrivateKeySigner = self.pk.parse().ok()?;
        Some(pk.address())
    }

    pub fn goerli() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Goerli,
            rpc: network_rpc_key("goerli")?,
            pk: network_private_key("goerli")?,
            etherscan: etherscan_key(NamedChain::Goerli)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn ftm_testnet() -> Option<Self> {
        Some(Self {
            chain: NamedChain::FantomTestnet,
            rpc: network_rpc_key("ftm_testnet")?,
            pk: network_private_key("ftm_testnet")?,
            etherscan: etherscan_key(NamedChain::FantomTestnet)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn optimism_kovan() -> Option<Self> {
        Some(Self {
            chain: NamedChain::OptimismKovan,
            rpc: network_rpc_key("op_kovan")?,
            pk: network_private_key("op_kovan")?,
            etherscan: etherscan_key(NamedChain::OptimismKovan)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn arbitrum_goerli() -> Option<Self> {
        Some(Self {
            chain: NamedChain::ArbitrumGoerli,
            rpc: network_rpc_key("arbitrum-goerli")?,
            pk: network_private_key("arbitrum-goerli")?,
            etherscan: etherscan_key(NamedChain::ArbitrumGoerli)?,
            verifier: "blockscout".to_string(),
        })
    }

    pub fn amoy() -> Option<Self> {
        Some(Self {
            chain: NamedChain::PolygonAmoy,
            rpc: network_rpc_key("amoy")?,
            pk: network_private_key("amoy")?,
            etherscan: etherscan_key(NamedChain::PolygonAmoy)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn sepolia_etherscan() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Sepolia,
            rpc: network_rpc_key("sepolia")?,
            pk: network_private_key("sepolia")?,
            etherscan: etherscan_key(NamedChain::Sepolia)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn sepolia_sourcify() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Sepolia,
            rpc: network_rpc_key("sepolia")?,
            pk: network_private_key("sepolia")?,
            etherscan: String::new(),
            verifier: "sourcify".to_string(),
        })
    }

    pub fn sepolia_sourcify_with_etherscan_api_key_set() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Sepolia,
            rpc: network_rpc_key("sepolia")?,
            pk: network_private_key("sepolia")?,
            etherscan: etherscan_key(NamedChain::Sepolia)?,
            verifier: "sourcify".to_string(),
        })
    }

    pub fn sepolia_blockscout() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Sepolia,
            rpc: network_rpc_key("sepolia")?,
            pk: network_private_key("sepolia")?,
            etherscan: String::new(),
            verifier: "blockscout".to_string(),
        })
    }

    pub fn sepolia_blockscout_with_etherscan_api_key_set() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Sepolia,
            rpc: network_rpc_key("sepolia")?,
            pk: network_private_key("sepolia")?,
            etherscan: etherscan_key(NamedChain::Sepolia)?,
            verifier: "blockscout".to_string(),
        })
    }

    pub fn sepolia_empty_verifier() -> Option<Self> {
        Some(Self {
            chain: NamedChain::Sepolia,
            rpc: network_rpc_key("sepolia")?,
            pk: network_private_key("sepolia")?,
            etherscan: String::new(),
            verifier: String::new(),
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

pub fn parse_verification_guid(out: &str) -> Option<String> {
    let mut lines = out.lines().map(str::trim).filter(|line| !line.is_empty());
    let line = lines.next()?;
    if lines.next().is_some() {
        return None;
    }
    let mut parts = line.split('\t').map(str::trim);
    let id = parts.next()?;
    let url = parts.next()?;
    if parts.next().is_some() || id.is_empty() || url.is_empty() {
        return None;
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    Some(id.to_string())
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
