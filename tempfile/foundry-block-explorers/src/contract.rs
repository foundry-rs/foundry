use crate::{
    serde_helpers::{deserialize_stringified_bool_or_u64, deserialize_stringified_u64},
    source_tree::{SourceTree, SourceTreeEntry},
    utils::{deserialize_address_opt, deserialize_source_code},
    Client, EtherscanError, Response, Result,
};
use alloy_json_abi::JsonAbi;
use alloy_primitives::{Address, Bytes, B256};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

#[cfg(feature = "foundry-compilers")]
use foundry_compilers::{
    artifacts::{EvmVersion, Settings},
    compilers::solc::SolcCompiler,
    solc::SolcSettings,
    ProjectBuilder, SolcConfig,
};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub enum SourceCodeLanguage {
    #[default]
    Solidity,
    Vyper,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceCodeEntry {
    pub content: String,
}

impl<T: Into<String>> From<T> for SourceCodeEntry {
    fn from(s: T) -> Self {
        Self { content: s.into() }
    }
}

/// The contract metadata's SourceCode field.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SourceCodeMetadata {
    /// Contains just mapped source code.
    // NOTE: this must come before `Metadata`
    Sources(HashMap<String, SourceCodeEntry>),
    /// Contains metadata and path mapped source code.
    Metadata {
        /// Programming language of the sources.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<SourceCodeLanguage>,
        /// Source path => source code
        #[serde(default)]
        sources: HashMap<String, SourceCodeEntry>,
        /// Compiler settings, None if the language is not Solidity.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settings: Option<serde_json::Value>,
    },
    /// Contains only the source code.
    SourceCode(String),
}

impl SourceCodeMetadata {
    pub fn source_code(&self) -> String {
        match self {
            Self::Metadata { sources, .. } | Self::Sources(sources) => {
                sources.values().map(|s| s.content.clone()).collect::<Vec<_>>().join("\n")
            }
            Self::SourceCode(s) => s.clone(),
        }
    }

    pub fn language(&self) -> Option<SourceCodeLanguage> {
        match self {
            Self::Metadata { language, .. } => *language,
            Self::Sources(_) => None,
            Self::SourceCode(_) => None,
        }
    }

    pub fn sources(&self) -> HashMap<String, SourceCodeEntry> {
        match self {
            Self::Metadata { sources, .. } => sources.clone(),
            Self::Sources(sources) => sources.clone(),
            Self::SourceCode(s) => HashMap::from([("Contract".into(), s.into())]),
        }
    }

    #[cfg(feature = "foundry-compilers")]
    pub fn settings(&self) -> Result<Option<Settings>> {
        match self {
            Self::Metadata { settings, .. } => match settings {
                Some(value) => {
                    if value.is_null() {
                        Ok(None)
                    } else {
                        let settings =
                            serde_json::from_value(value.to_owned()).map_err(|error| {
                                EtherscanError::Serde { error, content: value.to_string() }
                            })?;
                        Ok(Some(settings))
                    }
                }
                None => Ok(None),
            },
            Self::Sources(_) => Ok(None),
            Self::SourceCode(_) => Ok(None),
        }
    }

    #[cfg(not(feature = "foundry-compilers"))]
    pub fn settings(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Metadata { settings, .. } => settings.as_ref(),
            Self::Sources(_) => None,
            Self::SourceCode(_) => None,
        }
    }
}

/// Etherscan contract metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Metadata {
    /// Includes metadata for compiler settings and language.
    #[serde(deserialize_with = "deserialize_source_code")]
    pub source_code: SourceCodeMetadata,

    /// The ABI of the contract.
    #[serde(rename = "ABI")]
    pub abi: String,

    /// The name of the contract.
    pub contract_name: String,

    /// The version that this contract was compiled with. If it is a Vyper contract, it will start
    /// with "vyper:".
    pub compiler_version: String,

    /// Whether the optimizer was used. This value should only be 0 or 1.
    #[serde(deserialize_with = "deserialize_stringified_bool_or_u64")]
    pub optimization_used: u64,

    /// The number of optimizations performed.
    #[serde(deserialize_with = "deserialize_stringified_u64", alias = "OptimizationRuns", default)]
    pub runs: u64,

    /// The constructor arguments the contract was deployed with.
    #[serde(default)]
    pub constructor_arguments: Bytes,

    /// The version of the EVM the contract was deployed in. Can be either a variant of EvmVersion
    /// or "Default" which indicates the compiler's default.
    #[serde(rename = "EVMVersion")]
    pub evm_version: String,

    // ?
    #[serde(default)]
    pub library: String,

    /// The license of the contract.
    #[serde(default)]
    pub license_type: String,

    /// Whether this contract is a proxy. This value should only be 0 or 1.
    #[serde(deserialize_with = "deserialize_stringified_bool_or_u64", alias = "IsProxy")]
    pub proxy: u64,

    /// If this contract is a proxy, the address of its implementation.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_address_opt"
    )]
    pub implementation: Option<Address>,

    /// The swarm source of the contract.
    #[serde(default)]
    pub swarm_source: String,
}

impl Metadata {
    /// Returns the contract's source code.
    pub fn source_code(&self) -> String {
        self.source_code.source_code()
    }

    /// Returns the contract's programming language.
    pub fn language(&self) -> SourceCodeLanguage {
        self.source_code.language().unwrap_or_else(|| {
            if self.is_vyper() {
                SourceCodeLanguage::Vyper
            } else {
                SourceCodeLanguage::Solidity
            }
        })
    }

    /// Returns the contract's path mapped source code.
    pub fn sources(&self) -> HashMap<String, SourceCodeEntry> {
        self.source_code.sources()
    }

    /// Parses the ABI string into a [`JsonAbi`] struct.
    pub fn abi(&self) -> Result<JsonAbi> {
        serde_json::from_str(&self.abi)
            .map_err(|error| EtherscanError::Serde { error, content: self.abi.clone() })
    }

    /// Parses the compiler version.
    pub fn compiler_version(&self) -> Result<Version> {
        let v = &self.compiler_version;
        let v = v.strip_prefix("vyper:").unwrap_or(v);
        let v = v.strip_prefix('v').unwrap_or(v);
        match v.parse() {
            Err(e) => {
                let v = v.replace('a', "-alpha.");
                let v = v.replace('b', "-beta.");
                v.parse().map_err(|_| EtherscanError::Unknown(format!("bad compiler version: {e}")))
            }
            Ok(v) => Ok(v),
        }
    }

    /// Returns whether this contract is a Vyper or a Solidity contract.
    pub fn is_vyper(&self) -> bool {
        self.compiler_version.starts_with("vyper:")
    }

    /// Maps this contract's sources to a [SourceTreeEntry] vector.
    pub fn source_entries(&self) -> Vec<SourceTreeEntry> {
        let root = Path::new(&self.contract_name);
        self.sources()
            .into_iter()
            .map(|(path, entry)| {
                // This is relevant because the etherscan [Metadata](crate::contract::Metadata) can
                // contain absolute paths (supported by standard-json-input). See also: <https://github.com/foundry-rs/foundry/issues/6541>
                // for example, we want to ensure "/contracts/SimpleToken.sol" is mapped to
                // `<root_dir>/contracts/SimpleToken.sol`.
                let sanitized_path = crate::source_tree::sanitize_path(path);
                let path = root.join(sanitized_path);
                SourceTreeEntry { path, contents: entry.content }
            })
            .collect()
    }

    /// Returns the source tree of this contract's sources.
    pub fn source_tree(&self) -> SourceTree {
        SourceTree { entries: self.source_entries() }
    }

    /// Returns the contract's compiler settings.
    #[cfg(feature = "foundry-compilers")]
    pub fn settings(&self) -> Result<Settings> {
        let mut settings = self.source_code.settings()?.unwrap_or_default();

        if self.optimization_used == 1 && !settings.optimizer.enabled.unwrap_or_default() {
            settings.optimizer.enable();
            settings.optimizer.runs(self.runs as usize);
        }

        settings.evm_version = self.evm_version()?;

        Ok(settings)
    }

    /// Creates a Solc [ProjectBuilder] with this contract's settings.
    #[cfg(feature = "foundry-compilers")]
    pub fn project_builder(&self) -> Result<ProjectBuilder<SolcCompiler>> {
        let solc_config = SolcConfig::builder().settings(self.settings()?).build();

        Ok(ProjectBuilder::new(Default::default())
            .settings(SolcSettings { settings: solc_config, ..Default::default() }))
    }

    /// Parses the EVM version.
    #[cfg(feature = "foundry-compilers")]
    pub fn evm_version(&self) -> Result<Option<EvmVersion>> {
        match self.evm_version.to_lowercase().as_str() {
            "" | "default" => Ok(EvmVersion::default_version_solc(&self.compiler_version()?)),
            _ => {
                let evm_version = self
                    .evm_version
                    .parse()
                    .map_err(|e| EtherscanError::Unknown(format!("bad evm version: {e}")))?;
                Ok(Some(evm_version))
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContractMetadata {
    pub items: Vec<Metadata>,
}

impl IntoIterator for ContractMetadata {
    type Item = Metadata;
    type IntoIter = std::vec::IntoIter<Metadata>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl ContractMetadata {
    /// Returns the ABI of all contracts.
    pub fn abis(&self) -> Result<Vec<JsonAbi>> {
        self.items.iter().map(|c| c.abi()).collect()
    }

    /// Returns the combined source code of all contracts.
    pub fn source_code(&self) -> String {
        self.items.iter().map(|c| c.source_code()).collect::<Vec<_>>().join("\n")
    }

    /// Returns the combined [SourceTree] of all contracts.
    pub fn source_tree(&self) -> SourceTree {
        SourceTree { entries: self.items.iter().flat_map(|item| item.source_entries()).collect() }
    }
}

/// Contract creation data.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractCreationData {
    /// The contract's address.
    pub contract_address: Address,

    /// The contract's deployer address.
    /// NOTE: This field contains the address of an EOA that initiated the creation transaction.
    /// For contracts deployed by other contracts, the direct deployer address may vary.
    pub contract_creator: Address,

    /// The hash of the contract creation transaction.
    #[serde(rename = "txHash")]
    pub transaction_hash: B256,
}

impl Client {
    /// Fetches a verified contract's ABI.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse()?;
    /// let abi = client.contract_abi(address).await?;
    /// # Ok(()) }
    /// ```
    pub async fn contract_abi(&self, address: Address) -> Result<JsonAbi> {
        // apply caching
        if let Some(ref cache) = self.cache {
            // If this is None, then we have a cache miss
            if let Some(src) = cache.get_abi(address) {
                // If this is None, then the contract is not verified
                return match src {
                    Some(src) => Ok(src),
                    None => Err(EtherscanError::ContractCodeNotVerified(address)),
                };
            }
        }

        let query = self.create_query("contract", "getabi", HashMap::from([("address", address)]));
        let resp: Response<Option<String>> = self.get_json(&query).await?;

        let result = match resp.result {
            Some(result) => result,
            None => {
                if resp.message.contains("Contract source code not verified") {
                    return Err(EtherscanError::ContractCodeNotVerified(address));
                }
                return Err(EtherscanError::EmptyResult {
                    message: resp.message,
                    status: resp.status,
                });
            }
        };

        if resp.status == "0" && result.to_lowercase().contains("invalid api key") {
            return Err(EtherscanError::InvalidApiKey);
        }

        if result.starts_with("Max rate limit reached") {
            return Err(EtherscanError::RateLimitExceeded);
        }

        if result.starts_with("Contract source code not verified")
            || resp.message.starts_with("Contract source code not verified")
        {
            if let Some(ref cache) = self.cache {
                cache.set_abi(address, None);
            }
            return Err(EtherscanError::ContractCodeNotVerified(address));
        }
        let abi = serde_json::from_str(&result)
            .map_err(|error| EtherscanError::Serde { error, content: result })?;

        if let Some(ref cache) = self.cache {
            cache.set_abi(address, Some(&abi));
        }

        Ok(abi)
    }

    /// Fetches a contract's verified source code and its metadata.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse()?;
    /// let metadata = client.contract_source_code(address).await?;
    /// assert_eq!(metadata.items[0].contract_name, "DAO");
    /// # Ok(()) }
    /// ```
    pub async fn contract_source_code(&self, address: Address) -> Result<ContractMetadata> {
        // apply caching
        if let Some(ref cache) = self.cache {
            // If this is None, then we have a cache miss
            if let Some(src) = cache.get_source(address) {
                // If this is None, then the contract is not verified
                return match src {
                    Some(src) => Ok(src),
                    None => Err(EtherscanError::ContractCodeNotVerified(address)),
                };
            }
        }

        let query =
            self.create_query("contract", "getsourcecode", HashMap::from([("address", address)]));
        let response = self.get(&query).await?;

        // Source code is not verified
        if response.contains("Contract source code not verified") {
            if let Some(ref cache) = self.cache {
                cache.set_source(address, None);
            }
            return Err(EtherscanError::ContractCodeNotVerified(address));
        }

        let response: Response<ContractMetadata> = self.sanitize_response(response)?;
        let result = response.result;

        if let Some(ref cache) = self.cache {
            cache.set_source(address, Some(&result));
        }

        Ok(result)
    }

    /// Fetches a contract's creation transaction hash and deployer address.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn foo(client: foundry_block_explorers::Client) -> Result<(), Box<dyn std::error::Error>> {
    /// let address = "0xBB9bc244D798123fDe783fCc1C72d3Bb8C189413".parse()?;
    /// let creation_data = client.contract_creation_data(address).await?;
    /// let deployment_tx = creation_data.transaction_hash;
    /// let deployer = creation_data.contract_creator;
    /// # Ok(()) }
    /// ```
    pub async fn contract_creation_data(&self, address: Address) -> Result<ContractCreationData> {
        let query = self.create_query(
            "contract",
            "getcontractcreation",
            HashMap::from([("contractaddresses", address)]),
        );

        let response = self.get(&query).await?;

        // Address is not a contract or contract wasn't indexed yet
        if response.contains("No data found") {
            return Err(EtherscanError::ContractNotFound(address));
        }

        let response: Response<Vec<ContractCreationData>> = self.sanitize_response(response)?;

        // We are expecting the API to return exactly one result.
        let data = response.result.first().ok_or(EtherscanError::EmptyResult {
            message: response.message,
            status: response.status,
        })?;

        Ok(*data)
    }
}
