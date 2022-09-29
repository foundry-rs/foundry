//! Various helper functions

use ethers::prelude::Chain;

/// Returns the current millis since unix epoch.
///
/// This way we generate unique contracts so, etherscan will always have to verify them
pub fn millis_since_epoch() -> u128 {
    let now = std::time::SystemTime::now();
    now.duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|err| panic!("Current time {:?} is invalid: {:?}", now, err))
        .as_millis()
}

pub fn etherscan_key(chain: Chain) -> Option<String> {
    match chain {
        Chain::Fantom | Chain::FantomTestnet => {
            std::env::var("FTMSCAN_API_KEY").or_else(|_| std::env::var("FANTOMSCAN_API_KEY")).ok()
        }
        Chain::OptimismKovan => std::env::var("OP_KOVAN_API_KEY").ok(),
        _ => std::env::var("ETHERSCAN_API_KEY").ok(),
    }
}

pub fn network_rpc_key(chain: &str) -> Option<String> {
    let key = format!("{}_RPC_URL", chain.to_uppercase().replace("-", "_"));
    std::env::var(&key).ok()
}

pub fn network_private_key(chain: &str) -> Option<String> {
    let key = format!("{}_PRIVATE_KEY", chain.to_uppercase().replace("-", "_"));
    std::env::var(&key).or_else(|_| std::env::var("TEST_PRIVATE_KEY")).ok()
}

/// Represents external input required for executing verification requests
pub struct EnvExternalities {
    pub chain: Chain,
    pub rpc: String,
    pub pk: String,
    pub etherscan: String,
    pub verifier: String,
}

impl EnvExternalities {
    pub fn goerli() -> Option<Self> {
        Some(Self {
            chain: Chain::Goerli,
            rpc: network_rpc_key("goerli")?,
            pk: network_private_key("goerli")?,
            etherscan: etherscan_key(Chain::Goerli)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn ftm_testnet() -> Option<Self> {
        Some(Self {
            chain: Chain::FantomTestnet,
            rpc: network_rpc_key("ftm_testnet")?,
            pk: network_private_key("ftm_testnet")?,
            etherscan: etherscan_key(Chain::FantomTestnet)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn optimism_kovan() -> Option<Self> {
        Some(Self {
            chain: Chain::OptimismKovan,
            rpc: network_rpc_key("op_kovan")?,
            pk: network_private_key("op_kovan")?,
            etherscan: etherscan_key(Chain::OptimismKovan)?,
            verifier: "etherscan".to_string(),
        })
    }

    pub fn arbitrum_goerli() -> Option<Self> {
        Some(Self {
            chain: Chain::ArbitrumGoerli,
            rpc: network_rpc_key("arbitrum-goerli")?,
            pk: network_private_key("arbitrum-goerli")?,
            etherscan: etherscan_key(Chain::ArbitrumGoerli)?,
            verifier: "blockscout".to_string(),
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
            return Some(line.trim_start_matches("Deployed to: ").to_string())
        }
    }
    None
}

pub fn parse_verification_guid(out: &str) -> Option<String> {
    for line in out.lines() {
        if line.contains("GUID") {
            return Some(line.replace("GUID:", "").replace('`', "").trim().to_string())
        }
    }
    None
}
