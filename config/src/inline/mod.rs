mod conf_parser;
pub use conf_parser::{ConfParser, ConfParserError};
use ethers_solc::{artifacts::Node, ProjectCompileOutput};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// Represents per-test configurations, declared inline
/// as structured comments in Solidity test files. This allows
/// to create configs directly bound to solidity test.
/// `T` is the configuration type, and is bound to the [`ConfParser`] trait. Known
/// implementations of [`ConfParser`] include [`FuzzConfig`](super::FuzzConfig) and
/// [`InvariantConfig`](super::InvariantConfig))
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
    /// Returns an inline configuration, if any, for `contract_name` and `fn_name`. <br>
    /// - `contract_name` The name of the contract containing the annotated function. Note this
    ///   the actual contract name, not the solidity file name.
    /// - `fn_name` The name of the function annotated with an inline configuration.
    pub fn get_config<S: Into<String>>(&self, contract_name: S, fn_name: S) -> Option<&T> {
        self.configs.get(&(contract_name.into(), fn_name.into()))
    }
}

impl<'a, T> TryFrom<&'a ProjectCompileOutput> for InlineConfig<T>
where
    T: ConfParser + 'static,
{
    type Error = ConfParserError;

    /// Tries to detect inline configurations from the project compile output.
    /// This object is known to emit function comments, that we can try to parse
    /// at our convenience, to detect inline configurations.
    fn try_from(output: &'a ProjectCompileOutput) -> Result<Self, Self::Error> {
        let mut configs: HashMap<(String, String), T> = HashMap::new();

        for artifact in output.artifacts() {
            if let Some(ast) = artifact.1.ast.as_ref() {
                let contract_name: String = artifact.0;
                for node in ast.nodes.iter() {
                    try_apply(&mut configs, &contract_name, node)?;
                }
            }
        }

        Ok(Self { configs })
    }
}

/// Implements a DFS over a compiler output node and its children.
/// If a configuration is found for a solidity function, it is added to
/// `map` under the (contract name, function name) key.
/// This function may result in parsing errors (see [`ConfParserError`]).
fn try_apply<T>(
    map: &mut HashMap<(String, String), T>,
    contract_name: &str,
    node: &Node,
) -> Result<(), ConfParserError>
where
    T: ConfParser + 'static,
{
    for n in node.nodes.iter() {
        if let Some((fn_name, fn_docs)) = get_fn_data(n) {
            if let Some(config) = T::parse(fn_docs)? {
                let key = (contract_name.into(), fn_name);
                map.insert(key, config);
            }
        }

        try_apply(map, contract_name, n)?;
    }
    Ok(())
}

fn get_fn_data(node: &Node) -> Option<(String, String)> {
    if format!("{:?}", node.node_type) == *"FunctionDefinition" {
        let fn_data = &node.other;
        let fn_name: String = get_fn_name(fn_data)?;
        let fn_docs: String = get_fn_docs(fn_data)?;
        return Some((fn_name, fn_docs))
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
            return Some(comment.into())
        }
    }
    None
}
