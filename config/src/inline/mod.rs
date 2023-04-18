mod conf_parser;
pub use conf_parser::{ConfParser, ConfParserError};
use ethers_solc::{artifacts::Node, ProjectCompileOutput};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to solidity test.
/// `T` is the configuration type, and is bound to the [`ConfParser`] trait. Known
/// implementations of [`ConfParser`] include [`FuzzConfig`](super::FuzzConfig) and
/// [`InvariantConfig`](super::InvariantConfig))
#[derive(Default, Debug, Clone)]
pub struct InlineConfig<T>
where
    T: ConfParser + 'static,
{
    /// Maps a (contract, test-function)
    /// to a specific configuration provided by the user, in the
    /// test function comments.
    configs: HashMap<(String, String), T>,
}

impl<T> InlineConfig<T>
where
    T: ConfParser + 'static,
{
    /// Returns an inline configuration, if any, for `contract_id` and `fn_name`. <br>
    /// - `contract_id` The identifier of the contract containing the annotated function. Note that
    /// the contract id is a path relative to the project's root (i.e. someDirUnderRoot/Contract.t.sol:ContractName).
    ///   
    /// - `fn_name` The name of the function annotated with an inline configuration.
    pub fn get_config<S: Into<String>>(&self, contract_id: S, fn_name: S) -> Option<&T> {
        self.configs.get(&(contract_id.into(), fn_name.into()))
    }
}

impl<'a, T, P> TryFrom<(&'a ProjectCompileOutput, &'a T, &'a P)> for InlineConfig<T>
where
    T: ConfParser + 'static,
    P: AsRef<Path>,
{
    type Error = ConfParserError;

    /// Tries to create an instance of `Self`, detecting inline configurations from the project
    /// compile output. The second item of the input tuple is used as a base configuration.
    /// The aim is to inherit from "base" all the config parameters not explicitly derived from the
    /// compile output.
    ///
    /// NOTE: `ProjectCompileOutput` is known to emit function comments, that we can try to parse
    /// at our convenience, to detect inline test configurations.
    fn try_from(value: (&'a ProjectCompileOutput, &'a T, &'a P)) -> Result<Self, Self::Error> {
        let mut configs: HashMap<(String, String), T> = HashMap::new();
        let compile_output = value.0.clone();
        let base_conf = value.1;
        let root = value.2;

        for artifact in compile_output.with_stripped_file_prefixes(root).into_artifacts() {
            if let Some(ast) = artifact.1.ast.as_ref() {
                let contract_id: String = artifact.0.identifier();
                for node in ast.nodes.iter() {
                    try_apply(base_conf, &mut configs, &contract_id, node)?;
                }
            }
        }

        Ok(Self { configs })
    }
}

/// Implements a DFS over a compiler output node and its children.
/// If a configuration is found for a solidity function, it is added to
/// `map` under the (contract id, function name) key.
/// This function may result in parsing errors (see [`ConfParserError`]).
fn try_apply<T>(
    base_conf: &T,
    map: &mut HashMap<(String, String), T>,
    contract_id: &str,
    node: &Node,
) -> Result<(), ConfParserError>
where
    T: ConfParser + 'static,
{
    for n in node.nodes.iter() {
        if let Some((fn_name, fn_docs)) = get_fn_data(n) {
            let inline_conf = base_conf.try_merge(fn_docs)?;
            let key = (contract_id.into(), fn_name);
            map.insert(key, inline_conf);
        }

        try_apply(base_conf, map, contract_id, n)?;
    }
    Ok(())
}

fn get_fn_data(node: &Node) -> Option<(String, String)> {
    if format!("{:?}", node.node_type) == *"FunctionDefinition" {
        let fn_data = &node.other;
        let fn_name: String = get_fn_name(fn_data)?;
        let fn_docs: String = get_fn_docs(fn_data)?;
        return Some((fn_name, fn_docs));
    }
    None
}

fn get_fn_name(fn_data: &BTreeMap<String, Value>) -> Option<String> {
    match fn_data.get("name")? {
        Value::String(fn_name) => Some(fn_name.into()),
        _ => None,
    }
}

fn get_fn_docs(fn_data: &BTreeMap<String, Value>) -> Option<String> {
    if let Value::Object(fn_docs) = fn_data.get("documentation")? {
        if let Value::String(comment) = fn_docs.get("text")? {
            return Some(comment.into());
        }
    }
    None
}
