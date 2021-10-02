//! Helpers for etherscan.io
#![allow(unused)]
// TODO evaluate moving this to it's own crate eventually

use ethers::abi::Address;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap};

#[derive(Clone)]
pub struct Client {
    /// The client that executes the http requests
    client: reqwest::Client,
    /// The etherscan api key
    api_key: String,
    /// API endpoint like https://api(-chain).etherscan.io/api
    etherscan_api_url: Url,
    /// Base etherscan endpoint like https://etherscan.io/address
    etherscan_url: Url,
}

impl Client {
    /// Create a new client with the correct endpoints based on the chain.
    ///
    /// Supported chains are ethlive, mainnet,ropsten, kovan, rinkeby, goerli
    pub fn new(chain: &str, api_key: impl Into<String>) -> eyre::Result<Self> {
        let (etherscan_api_url, etherscan_url) = match chain {
            "ethlive" | "mainnet" => {
                (
                    Url::parse("https://api.etherscan.io/api"),
                    Url::parse("https://etherscan.io/address"),
                )
            },
            "ropsten"|"kovan"|"rinkeby"|"goerli" => {
                (
                    Url::parse(&format!("https://api-{}.etherscan.io/api", chain)),
                    Url::parse(&format!("https://{}.etherscan.io/address", chain)),
                )
            }
            s => {
                return Err(
                    eyre::eyre!("Verification only works on mainnet, ropsten, kovan, rinkeby, and goerli, found `{}` chain", s)
                )
            }
        };
        Ok(Self {
            client: Default::default(),
            api_key: api_key.into(),
            etherscan_api_url: etherscan_api_url.expect("is valid http"),
            etherscan_url: etherscan_url.expect("is valid http"),
        })
    }

    fn body<T: Serialize>(
        &self,
        module: &'static str,
        action: &'static str,
        other: T,
    ) -> PostBody<T> {
        PostBody {
            apikey: Cow::Borrowed(&self.api_key),
            module: Cow::Borrowed(module),
            action: Cow::Borrowed(action),
            other,
        }
    }

    /// Submit Source Code for Verification
    pub async fn submit_contract_verification(
        &self,
        contract: VerifyContract,
    ) -> eyre::Result<Response> {
        let body = self.body("contract", "verifysourcecode", contract);
        Ok(self
            .client
            .post(self.etherscan_api_url.clone())
            .json(&body)
            .send()
            .await?
            .json()
            .await?)
    }

    /// Check Source Code Verification Status with receipt received from
    /// `[Self::submit_contract_verification]`
    pub async fn check_verify_status(&self, guid: impl AsRef<str>) -> eyre::Result<Response> {
        let mut map = HashMap::new();
        map.insert("guid", guid.as_ref());
        let body = self.body("contract", "checkverifystatus", map);
        Ok(self
            .client
            .post(self.etherscan_api_url.clone())
            .json(&body)
            .send()
            .await?
            .json()
            .await?)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    pub status: String,
    pub message: String,
    pub result: String,
}

#[derive(Debug, Serialize)]
struct PostBody<'a, T: Serialize> {
    apikey: Cow<'a, str>,
    module: Cow<'a, str>,
    action: Cow<'a, str>,
    #[serde(flatten)]
    other: T,
}

/// Arguments for verifying contracts
#[derive(Debug, Clone, Serialize)]
pub struct VerifyContract {
    pub address: Address,
    pub source: String,
    #[serde(rename = "codeformat")]
    pub code_format: CodeFormat,
    /// if codeformat=solidity-standard-json-input, then expected as `erc20.sol:erc20`
    #[serde(rename = "contractname", skip_serializing_if = "Option::is_none")]
    pub contract_name: Option<String>,
    #[serde(rename = "compilerversion")]
    pub compiler_version: String,
    /// applicable when codeformat=solidity-single-file
    #[serde(rename = "optimizationUsed", skip_serializing_if = "Option::is_none")]
    optimization_used: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<u32>,
    /// NOTE: there is a typo in the etherscan API `constructorArguements`
    #[serde(rename = "constructorArguements", skip_serializing_if = "Option::is_none")]
    pub constructor_arguments: Option<String>,
    #[serde(flatten)]
    pub other: HashMap<String, String>,
}

impl VerifyContract {
    pub fn new(address: Address, source: String, compilerversion: String) -> Self {
        Self {
            address,
            source,
            code_format: Default::default(),
            contract_name: None,
            compiler_version: compilerversion,
            optimization_used: None,
            runs: None,
            constructor_arguments: None,
            other: Default::default(),
        }
    }

    pub fn contract_name(mut self, name: impl Into<String>) -> Self {
        self.contract_name = Some(name.into());
        self
    }

    pub fn runs(mut self, runs: u32) -> Self {
        self.runs = Some(runs);
        self
    }

    pub fn optimization(mut self, optimization: bool) -> Self {
        if optimization {
            self.optimized()
        } else {
            self.not_optimized()
        }
    }

    pub fn optimized(mut self) -> Self {
        self.optimization_used = Some(1);
        self
    }

    pub fn not_optimized(mut self) -> Self {
        self.optimization_used = Some(0);
        self
    }

    pub fn code_format(mut self, code_format: CodeFormat) -> Self {
        self.code_format = code_format;
        self
    }

    pub fn constructor_arguments(
        mut self,
        constructor_arguments: Option<impl Into<String>>,
    ) -> Self {
        self.constructor_arguments =
            constructor_arguments.map(|s| s.into().trim_start_matches("0x").to_string());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CodeFormat {
    #[serde(rename = "solidity-single-file")]
    SingleFile,
    #[serde(rename = "solidity-standard-json-inpu")]
    StandardJsonInput,
}

impl AsRef<str> for CodeFormat {
    fn as_ref(&self) -> &str {
        match self {
            CodeFormat::SingleFile => "solidity-single-file",
            CodeFormat::StandardJsonInput => "solidity-standard-json-input",
        }
    }
}

impl Default for CodeFormat {
    fn default() -> Self {
        CodeFormat::SingleFile
    }
}
