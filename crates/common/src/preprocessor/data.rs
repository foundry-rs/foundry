use super::span_to_range;
use foundry_compilers::artifacts::{Source, Sources};
use path_slash::PathExt;
use solar_parse::interface::{Session, SourceMap};
use solar_sema::{
    hir::{Contract, ContractId, Hir},
    interface::source_map::FileName,
};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

/// Keeps data about project contracts definitions referenced from tests and scripts.
/// Contract id -> Contract data definition mapping.
pub type PreprocessorData = BTreeMap<ContractId, ContractData>;

/// Collects preprocessor data from referenced contracts.
pub(crate) fn collect_preprocessor_data(
    sess: &Session,
    hir: &Hir<'_>,
    referenced_contracts: &HashSet<ContractId>,
) -> PreprocessorData {
    let mut data = PreprocessorData::default();
    for contract_id in referenced_contracts {
        let contract = hir.contract(*contract_id);
        let source = hir.source(contract.source);

        let FileName::Real(path) = &source.file.name else {
            continue;
        };

        let contract_data =
            ContractData::new(hir, *contract_id, contract, path, source, sess.source_map());
        data.insert(*contract_id, contract_data);
    }
    data
}

/// Creates helper libraries for contracts with a non-empty constructor.
///
/// See [`ContractData::build_helper`] for more details.
pub(crate) fn create_deploy_helpers(data: &BTreeMap<ContractId, ContractData>) -> Sources {
    let mut deploy_helpers = Sources::new();
    for (contract_id, contract) in data {
        if let Some(code) = contract.build_helper() {
            let path = format!("foundry-pp/DeployHelper{}.sol", contract_id.get());
            deploy_helpers.insert(path.into(), Source::new(code));
        }
    }
    deploy_helpers
}

/// Keeps data about a contract constructor.
#[derive(Debug)]
pub struct ContractConstructorData {
    /// ABI encoded args.
    pub abi_encode_args: String,
    /// Constructor struct fields.
    pub struct_fields: String,
}

/// Keeps data about a single contract definition.
#[derive(Debug)]
pub(crate) struct ContractData {
    /// HIR Id of the contract.
    contract_id: ContractId,
    /// Path of the source file.
    path: PathBuf,
    /// Name of the contract
    name: String,
    /// Constructor parameters, if any.
    pub constructor_data: Option<ContractConstructorData>,
    /// Artifact string to pass into cheatcodes.
    pub artifact: String,
}

impl ContractData {
    fn new(
        hir: &Hir<'_>,
        contract_id: ContractId,
        contract: &Contract<'_>,
        path: &Path,
        source: &solar_sema::hir::Source<'_>,
        source_map: &SourceMap,
    ) -> Self {
        let artifact = format!("{}:{}", path.to_slash_lossy(), contract.name);

        // Process data for contracts with constructor and parameters.
        let constructor_data = contract
            .ctor
            .map(|ctor_id| hir.function(ctor_id))
            .filter(|ctor| !ctor.parameters.is_empty())
            .map(|ctor| {
                let mut abi_encode_args = vec![];
                let mut struct_fields = vec![];
                let mut arg_index = 0;
                for param_id in ctor.parameters {
                    let src = source.file.src.as_str();
                    let loc = span_to_range(source_map, hir.variable(*param_id).span);
                    let mut new_src = src[loc].replace(" memory ", " ").replace(" calldata ", " ");
                    if let Some(ident) = hir.variable(*param_id).name {
                        abi_encode_args.push(format!("args.{}", ident.name));
                    } else {
                        // Generate an unique name if constructor arg doesn't have one.
                        arg_index += 1;
                        abi_encode_args.push(format!("args.foundry_pp_ctor_arg{arg_index}"));
                        new_src.push_str(&format!(" foundry_pp_ctor_arg{arg_index}"));
                    }
                    struct_fields.push(new_src);
                }

                ContractConstructorData {
                    abi_encode_args: abi_encode_args.join(", "),
                    struct_fields: struct_fields.join("; "),
                }
            });

        Self {
            contract_id,
            path: path.to_path_buf(),
            name: contract.name.to_string(),
            constructor_data,
            artifact,
        }
    }

    /// If contract has a non-empty constructor, generates a helper source file for it containing a
    /// helper to encode constructor arguments.
    ///
    /// This is needed because current preprocessing wraps the arguments, leaving them unchanged.
    /// This allows us to handle nested new expressions correctly. However, this requires us to have
    /// a way to wrap both named and unnamed arguments. i.e you can't do abi.encode({arg: val}).
    ///
    /// This function produces a helper struct + a helper function to encode the arguments. The
    /// struct is defined in scope of an abstract contract inheriting the contract containing the
    /// constructor. This is done as a hack to allow us to inherit the same scope of definitions.
    ///
    /// The resulted helper looks like this:
    /// ```solidity
    /// import "lib/openzeppelin-contracts/contracts/token/ERC20.sol";
    ///
    /// abstract contract DeployHelper335 is ERC20 {
    ///     struct FoundryPpConstructorArgs {
    ///         string name;
    ///         string symbol;
    ///     }
    /// }
    ///
    /// function encodeArgs335(DeployHelper335.FoundryPpConstructorArgs memory args) pure returns (bytes memory) {
    ///     return abi.encode(args.name, args.symbol);
    /// }
    /// ```
    ///
    /// Example usage:
    /// ```solidity
    /// new ERC20(name, symbol)
    /// ```
    /// becomes
    /// ```solidity
    /// vm.deployCode("artifact path", encodeArgs335(DeployHelper335.FoundryPpConstructorArgs(name, symbol)))
    /// ```
    /// With named arguments:
    /// ```solidity
    /// new ERC20({name: name, symbol: symbol})
    /// ```
    /// becomes
    /// ```solidity
    /// vm.deployCode("artifact path", encodeArgs335(DeployHelper335.FoundryPpConstructorArgs({name: name, symbol: symbol})))
    /// ```
    pub fn build_helper(&self) -> Option<String> {
        let Self { contract_id, path, name, constructor_data, artifact: _ } = self;

        let Some(constructor_details) = constructor_data else { return None };
        let contract_id = contract_id.get();
        let struct_fields = &constructor_details.struct_fields;
        let abi_encode_args = &constructor_details.abi_encode_args;

        let helper = format!(
            r#"
// SPDX-License-Identifier: MIT
pragma solidity >=0.4.0;

import "{path}";

abstract contract DeployHelper{contract_id} is {name} {{
    struct FoundryPpConstructorArgs {{
        {struct_fields};
    }}
}}

function encodeArgs{contract_id}(DeployHelper{contract_id}.FoundryPpConstructorArgs memory args) pure returns (bytes memory) {{
    return abi.encode({abi_encode_args});
}}
        "#,
            path = path.to_slash_lossy(),
        );

        Some(helper)
    }
}
