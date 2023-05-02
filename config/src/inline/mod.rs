mod conf_parser;
pub use conf_parser::{InlineConfigParser, InlineConfigParserError};
use ethers_solc::{
    artifacts::{ast::NodeType, Node},
    ProjectCompileOutput,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

/// Wrapper error struct that catches config parsing
/// errors [`InlineConfigParserError`], enriching them with context information
/// reporting the misconfigured line.
#[derive(thiserror::Error, Debug)]
#[error("Inline config error detected at {line} {source}")]
pub struct InlineConfigError {
    /// Specifies the misconfigured line. This is something of the form
    /// `dir/TestContract.t.sol:FuzzContract:10:12:111`
    line: String,
    source: InlineConfigParserError,
}

/// Represents a (test-contract, test-function) pair
type InlineConfigKey = (String, String);

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to a solidity test.
/// `T` is the configuration type, and is bound to the [`InlineConfigParser`] trait. Known
/// implementations of [`InlineConfigParser`] include [`FuzzConfig`](super::FuzzConfig) and
/// [`InvariantConfig`](super::InvariantConfig))
#[derive(Default, Debug, Clone)]
pub struct InlineConfig<T>
where
    T: InlineConfigParser + 'static,
{
    /// Maps a (test-contract, test-function) pair
    /// to a specific configuration provided by the user.
    configs: HashMap<InlineConfigKey, T>,
}

impl<T> InlineConfig<T>
where
    T: InlineConfigParser + 'static,
{
    /// Returns an inline configuration, if any, for `contract_id` and `fn_name`. <br>
    /// - `contract_id` The identifier of the contract containing the annotated function. Note that
    /// contract id is a path relative to the project's root check `with_stripped_file_prefixes`
    /// in [`ProjectCompileOutput`](ethers_solc::compile::output::ProjectCompileOutput)).
    /// - `fn_name` The name of the function annotated with an inline configuration.
    pub fn get_config<S: Into<String>>(&self, contract_id: S, fn_name: S) -> Option<&T> {
        self.configs.get(&(contract_id.into(), fn_name.into()))
    }
}

impl<'a, T, P> TryFrom<(&'a ProjectCompileOutput, &'a T, &'a P)> for InlineConfig<T>
where
    T: InlineConfigParser + 'static,
    P: AsRef<Path>,
{
    type Error = InlineConfigError;

    /// Tries to create an instance of `Self`, detecting inline configurations from the project
    /// compile output.
    ///
    /// Param is a tuple, whose elements are:
    /// 1. Solidity compiler output, essential to extract comments from compiled functions.
    /// 2. A reference to a base configuration. This essentially works as a fallback.
    /// 3. A root path to express contract base dirs. This is essential to match inline configs at
    /// runtime.
    fn try_from(value: (&'a ProjectCompileOutput, &'a T, &'a P)) -> Result<Self, Self::Error> {
        let mut configs: HashMap<(String, String), T> = HashMap::new();
        let compile_output = value.0.clone();
        let base_conf = value.1;
        let root = value.2;

        for artifact in compile_output.with_stripped_file_prefixes(root).into_artifacts() {
            if let Some(ast) = artifact.1.ast.as_ref() {
                let contract_id: String = artifact.0.identifier();
                if let Some(node) = contract_root_node(&ast.nodes, &contract_id) {
                    try_apply(base_conf, &mut configs, &contract_id, node)?;
                }
            }
        }

        Ok(Self { configs })
    }
}

/// Given a list of nodes, find a "ContractDefinition" node that matches
/// the provided contract_id.
fn contract_root_node<'a>(nodes: &'a [Node], contract_id: &'a str) -> Option<&'a Node> {
    for n in nodes.iter() {
        if let NodeType::ContractDefinition = n.node_type {
            let contract_data = &n.other;
            if let Value::String(contract_name) = contract_data.get("name")? {
                if contract_id.ends_with(contract_name) {
                    return Some(n)
                }
            }
        }
    }
    None
}

/// Implements a DFS over a compiler output node and its children.
/// If a configuration is found for a solidity function, it is added to
/// `map` under the (contract id, function name) key.
/// This function may result in parsing errors (see [`InlineConfigParserError`]).
fn try_apply<T>(
    base_conf: &T,
    map: &mut HashMap<(String, String), T>,
    contract_id: &str,
    node: &Node,
) -> Result<(), InlineConfigError>
where
    T: InlineConfigParser + 'static,
{
    for n in node.nodes.iter() {
        if let Some((fn_name, fn_docs, docs_src_line)) = get_fn_data(n) {
            match base_conf.try_merge(fn_docs.clone()) {
                Ok(Some(conf)) => {
                    // We found a config that applies for the pair contract_id, fn_name.
                    let key: InlineConfigKey = (contract_id.into(), fn_name);
                    map.insert(key, conf);
                }
                Err(e) => {
                    // We add context information to the error, to specify the misconfigured line
                    let line = format!("{contract_id}:{fn_name}:{docs_src_line}");
                    Err(InlineConfigError { line, source: e })?
                }
                _ => { /* No inline config found, do nothing */ }
            }
        }

        try_apply(base_conf, map, contract_id, n)?;
    }
    Ok(())
}

fn get_fn_data(node: &Node) -> Option<(String, String, String)> {
    if let NodeType::FunctionDefinition = node.node_type {
        let fn_data = &node.other;
        let fn_name: String = get_fn_name(fn_data)?;
        let (fn_docs, docs_src_line): (String, String) = get_fn_docs(fn_data)?;
        return Some((fn_name, fn_docs, docs_src_line))
    }

    None
}

fn get_fn_name(fn_data: &BTreeMap<String, Value>) -> Option<String> {
    match fn_data.get("name")? {
        Value::String(fn_name) => Some(fn_name.into()),
        _ => None,
    }
}

/// Inspects compiler output for a test function and returns:
/// - `Some((String, String))` in case the function has natspec comments. First item is a textual
///   natspec representation, the second item is the natspec src line, in the form "raw:col:length".
/// - `None` in case the function has not natspec comments.
fn get_fn_docs(fn_data: &BTreeMap<String, Value>) -> Option<(String, String)> {
    if let Value::Object(fn_docs) = fn_data.get("documentation")? {
        if let Value::String(comment) = fn_docs.get("text")? {
            let src_line = fn_docs
                .get("src")
                .map(Value::to_string)
                .unwrap_or(String::from("<no-src-line-available>"));
            return Some((comment.into(), src_line))
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::num::ParseIntError;

    use crate::InlineConfigError;

    use super::InlineConfigParserError;

    #[test]
    fn inline_config_error() {
        let source =
            InlineConfigParserError::ParseBoolError("key".into(), "invalid-bool-value".into());
        let line = "dir/TestContract.t.sol:FuzzContract:10:12:111".to_string();
        let error = InlineConfigError { line: line.clone(), source: source.clone() };

        let expected = format!("Inline config error detected at {line} {source}");
        assert_eq!(error.to_string(), expected);
    }
}
