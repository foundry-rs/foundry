use crate::{Client, Response, Result};
use alloy_primitives::Address;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Arguments for verifying contracts
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct VerifyContract {
    #[serde(rename = "contractaddress")]
    pub address: Address,
    #[serde(rename = "sourceCode")]
    pub source: String,
    #[serde(rename = "codeformat")]
    pub code_format: CodeFormat,
    /// if codeformat=solidity-standard-json-input, then expected as
    /// `erc20.sol:erc20`
    #[serde(rename = "contractname")]
    pub contract_name: String,
    #[serde(rename = "compilerversion")]
    pub compiler_version: String,
    /// applicable when codeformat=solidity-single-file
    #[serde(rename = "optimizationUsed", skip_serializing_if = "Option::is_none")]
    pub optimization_used: Option<String>,
    /// applicable when codeformat=solidity-single-file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runs: Option<String>,
    /// The constructor arguments for the contract, if any.
    ///
    /// NOTE: This is renamed as the misspelled `ethers-etherscan/src/verify.rs`. The reason for
    /// this is that Etherscan has had this misspelling on their API for quite a long time, and
    /// changing it would break verification with arguments.
    ///
    /// For instances (e.g. blockscout) that might support the proper spelling, the field
    /// `blockscout_constructor_arguments` is populated with the exact arguments passed to this
    /// field as well.
    #[serde(rename = "constructorArguements", skip_serializing_if = "Option::is_none")]
    pub constructor_arguments: Option<String>,
    /// Properly spelled constructor arguments. This is needed as some blockscout instances
    /// can identify the correct spelling instead of the misspelled version above.
    #[serde(rename = "constructorArguments", skip_serializing_if = "Option::is_none")]
    pub blockscout_constructor_arguments: Option<String>,
    /// applicable when codeformat=solidity-single-file
    #[serde(rename = "evmversion", skip_serializing_if = "Option::is_none")]
    pub evm_version: Option<String>,
    /// Use `--via-ir`.
    #[serde(rename = "viaIR", skip_serializing_if = "Option::is_none")]
    pub via_ir: Option<bool>,
    #[serde(flatten)]
    pub other: HashMap<String, String>,
}

impl VerifyContract {
    pub fn new(
        address: Address,
        contract_name: String,
        source: String,
        compiler_version: String,
    ) -> Self {
        Self {
            address,
            source,
            code_format: Default::default(),
            contract_name,
            compiler_version,
            optimization_used: None,
            runs: None,
            constructor_arguments: None,
            blockscout_constructor_arguments: None,
            evm_version: None,
            via_ir: None,
            other: Default::default(),
        }
    }

    pub fn runs(mut self, runs: u32) -> Self {
        self.runs = Some(format!("{runs}"));
        self
    }

    pub fn optimization(self, optimization: bool) -> Self {
        if optimization {
            self.optimized()
        } else {
            self.not_optimized()
        }
    }

    pub fn optimized(mut self) -> Self {
        self.optimization_used = Some("1".to_string());
        self
    }

    pub fn not_optimized(mut self) -> Self {
        self.optimization_used = Some("0".to_string());
        self
    }

    pub fn code_format(mut self, code_format: CodeFormat) -> Self {
        self.code_format = code_format;
        self
    }

    pub fn evm_version(mut self, evm_version: impl Into<String>) -> Self {
        self.evm_version = Some(evm_version.into());
        self
    }

    pub fn via_ir(mut self, via_ir: bool) -> Self {
        self.via_ir = Some(via_ir);
        self
    }

    pub fn constructor_arguments(
        mut self,
        constructor_arguments: Option<impl Into<String>>,
    ) -> Self {
        let constructor_args = constructor_arguments.map(|s| {
            s.into()
                .trim()
                // TODO is this correct?
                .trim_start_matches("0x")
                .to_string()
        });
        self.constructor_arguments.clone_from(&constructor_args);
        self.blockscout_constructor_arguments = constructor_args;
        self
    }
}

/// Arguments for verifying a proxy contract
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_copy_implementations)]
pub struct VerifyProxyContract {
    /// Proxy contract's address
    pub address: Address,
    /// Implementation contract proxy points to - must be verified before call.
    #[serde(default, rename = "expectedimplementation", skip_serializing_if = "Option::is_none")]
    pub expected_impl: Option<Address>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeFormat {
    #[serde(rename = "solidity-single-file")]
    SingleFile,

    #[default]
    #[serde(rename = "solidity-standard-json-input")]
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

impl Client {
    /// Submit Source Code for Verification
    pub async fn submit_contract_verification(
        &self,
        contract: &VerifyContract,
    ) -> Result<Response<String>> {
        let body = self.create_query("contract", "verifysourcecode", contract);
        self.post_form(&body).await
    }

    /// Check Source Code Verification Status with receipt received from
    /// `[Self::submit_contract_verification]`
    pub async fn check_contract_verification_status(
        &self,
        guid: impl AsRef<str>,
    ) -> Result<Response<String>> {
        let body = self.create_query(
            "contract",
            "checkverifystatus",
            HashMap::from([("guid", guid.as_ref())]),
        );
        self.post_form(&body).await
    }

    /// Submit Proxy Contract for Verification
    pub async fn submit_proxy_contract_verification(
        &self,
        contract: &VerifyProxyContract,
    ) -> Result<Response<String>> {
        let body = self.create_query("contract", "verifyproxycontract", contract);
        self.post_form(&body).await
    }

    /// Check Proxy Contract Verification Status with receipt received from
    /// `[Self::submit_proxy_contract_verification]`
    pub async fn check_proxy_contract_verification_status(
        &self,
        guid: impl AsRef<str>,
    ) -> Result<Response<String>> {
        let body = self.create_query(
            "contract",
            "checkproxyverification",
            HashMap::from([("guid", guid.as_ref())]),
        );
        self.post_form(&body).await
    }
}
