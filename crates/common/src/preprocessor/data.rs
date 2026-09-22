use super::span_to_range;
use foundry_compilers::artifacts::{Source, Sources};
use path_slash::PathExt;
use solar::sema::{
    Gcx,
    hir::{Contract, ContractId},
    interface::source_map::FileName,
};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

/// Keeps data about project contracts definitions referenced from tests and scripts.
/// Contract id -> Contract data definition mapping.
pub type PreprocessorData = BTreeMap<ContractId, ContractData>;

/// Keeps data about a contract constructor.
#[derive(Debug)]
pub struct ContractConstructorData {
    /// ABI encoded args.
    pub abi_encode_args: String,
    /// Constructor struct fields.
    pub struct_fields: String,
    /// Generated helper contract identifier.
    pub helper_contract: String,
    /// Generated constructor argument struct identifier.
    pub args_struct: String,
    /// Generated ABI encoding function identifier.
    pub encode_function: String,
    /// Generated helper source-unit path.
    pub helper_path: PathBuf,
}

/// Keeps data about a single contract definition.
#[derive(Debug)]
pub(crate) struct ContractData {
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
        gcx: Gcx<'_>,
        contract_id: ContractId,
        contract: &Contract<'_>,
        path: &Path,
        source: &solar::sema::hir::Source<'_>,
        reserved_identifiers: &str,
        source_units: &[PathBuf],
    ) -> Self {
        let artifact =
            solidity_string_content(&format!("{}:{}", path.to_slash_lossy(), contract.name));

        // Process data for contracts with constructor and parameters.
        let constructor_data = contract
            .ctor
            .map(|ctor_id| gcx.hir.function(ctor_id))
            .filter(|ctor| !ctor.parameters.is_empty())
            .map(|ctor| {
                let contract_id = contract_id.index();
                let mut abi_encode_args = vec![];
                let mut struct_fields = vec![];
                let mut arg_index = 0;
                for param_id in ctor.parameters {
                    let param = gcx.hir.variable(*param_id);
                    let loc = span_to_range(gcx.sess.source_map(), param.ty.span);
                    let ty = &source.file.src[loc];
                    let name = if let Some(ident) = param.name {
                        ident.name.to_string()
                    } else {
                        // Generate a unique name if the constructor arg does not have one.
                        arg_index += 1;
                        unique_identifier(
                            reserved_identifiers,
                            format!("foundry_pp_ctor_arg{arg_index}"),
                        )
                    };
                    abi_encode_args.push(format!("args.{name}"));
                    struct_fields.push(format!("{ty} {name}"));
                }

                ContractConstructorData {
                    abi_encode_args: abi_encode_args.join(", "),
                    struct_fields: struct_fields.join("; "),
                    helper_contract: unique_identifier(
                        reserved_identifiers,
                        format!("DeployHelper{contract_id}"),
                    ),
                    args_struct: unique_identifier(
                        reserved_identifiers,
                        "FoundryPpConstructorArgs".to_string(),
                    ),
                    encode_function: unique_identifier(
                        reserved_identifiers,
                        format!("encodeArgs{contract_id}"),
                    ),
                    helper_path: deploy_helper_path(contract_id, source_units),
                }
            });

        Self {
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
        let Self { path, name, constructor_data, artifact: _ } = self;

        let Some(constructor_details) = constructor_data else { return None };
        let struct_fields = &constructor_details.struct_fields;
        let abi_encode_args = &constructor_details.abi_encode_args;
        let helper_contract = &constructor_details.helper_contract;
        let args_struct = &constructor_details.args_struct;
        let encode_function = &constructor_details.encode_function;

        let path = solidity_string_content(path.to_slash_lossy().as_ref());
        let helper = format!(
            r#"
// SPDX-License-Identifier: MIT
pragma solidity >=0.4.0;

import "{path}";

abstract contract {helper_contract} is {name} {{
    struct {args_struct} {{
        {struct_fields};
    }}
}}

function {encode_function}({helper_contract}.{args_struct} memory args) pure returns (bytes memory) {{
    return abi.encode({abi_encode_args});
}}
        "#,
        );

        Some(helper)
    }
}

fn unique_identifier(source: &str, mut identifier: String) -> String {
    while source.contains(&identifier) {
        identifier.push('_');
    }
    identifier
}

fn solidity_string_content(value: &str) -> String {
    value.chars().fold(String::new(), |mut escaped, char| {
        match char {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            char => escaped.push(char),
        }
        escaped
    })
}

/// Collects preprocessor data from referenced contracts.
pub(crate) fn collect_preprocessor_data(
    gcx: Gcx<'_>,
    referenced_contracts: &HashSet<ContractId>,
    root_dir: &Path,
    source_units: &[PathBuf],
) -> PreprocessorData {
    let mut data = PreprocessorData::default();
    let reserved_identifiers = gcx
        .hir
        .source_ids()
        .map(|source_id| gcx.hir.source(source_id).file.src.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for contract_id in referenced_contracts {
        let contract = gcx.hir.contract(*contract_id);
        let source = gcx.hir.source(contract.source);

        let FileName::Real(path) = &source.file.name else {
            continue;
        };

        // Match the compiler input paths in generated imports and artifact references.
        let path = path.strip_prefix(root_dir).unwrap_or(path);
        let contract_data = ContractData::new(
            gcx,
            *contract_id,
            contract,
            path,
            source,
            &reserved_identifiers,
            source_units,
        );
        data.insert(*contract_id, contract_data);
    }
    data
}

/// Creates helper libraries for contracts with a non-empty constructor.
///
/// See [`ContractData::build_helper`] for more details.
pub(crate) fn create_deploy_helpers(data: &BTreeMap<ContractId, ContractData>) -> Sources {
    let mut deploy_helpers = Sources::new();
    for contract in data.values() {
        if let Some(code) = contract.build_helper() {
            let path = contract.constructor_data.as_ref().unwrap().helper_path.clone();
            deploy_helpers.insert(path, Source::new(code));
        }
    }
    deploy_helpers
}

/// Returns a generated helper path that cannot overwrite an existing source unit.
pub(crate) fn deploy_helper_path(contract_id: usize, source_units: &[PathBuf]) -> PathBuf {
    let mut stem = format!("DeployHelper{contract_id}");
    loop {
        let path = PathBuf::from(format!("foundry-pp/{stem}.sol"));
        if !source_units.iter().any(|source_unit| source_unit == &path) {
            return path;
        }
        stem.push('_');
    }
}

#[cfg(test)]
mod tests {
    use super::deploy_helper_path;
    use std::path::PathBuf;

    #[test]
    fn deploy_helper_path_does_not_replace_source_units() {
        let source_units = [
            PathBuf::from("foundry-pp/DeployHelper7.sol"),
            PathBuf::from("foundry-pp/DeployHelper7_.sol"),
        ];

        assert_eq!(
            deploy_helper_path(7, &source_units),
            PathBuf::from("foundry-pp/DeployHelper7__.sol")
        );
    }
}
