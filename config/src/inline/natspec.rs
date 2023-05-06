use std::{collections::BTreeMap, path::Path};

use ethers_solc::{
    artifacts::{ast::NodeType, Node},
    ProjectCompileOutput,
};
use serde_json::Value;

use super::{remove_whitespaces, INLINE_CONFIG_PREFIX, INLINE_CONFIG_PREFIX_SELECTED_PROFILE};

/// Convenient struct to hold in-line per-test configurations
pub struct NatSpec {
    /// The parent contract of the natspec
    pub contract: String,
    /// The function annotated with the natspec
    pub function: String,
    /// The line the natspec appears, in the form
    /// `row:col:length` i.e. `10:21:122`
    pub line: String,
    /// The actual natspec comment, without slashes or block
    /// punctuation
    pub docs: String,
}

impl NatSpec {
    /// Factory function that extracts a vector of [`NatSpec`] instances from
    /// a solc compiler output. The root path is to express contract base dirs.
    /// That is essential to match per-test configs at runtime.
    pub fn parse<P>(output: &ProjectCompileOutput, root: &P) -> Vec<Self>
    where
        P: AsRef<Path>,
    {
        let mut natspecs: Vec<Self> = vec![];

        let output = output.clone();
        for artifact in output.with_stripped_file_prefixes(root).into_artifacts() {
            if let Some(ast) = artifact.1.ast.as_ref() {
                let contract: String = artifact.0.identifier();
                if let Some(node) = contract_root_node(&ast.nodes, &contract) {
                    apply(&mut natspecs, &contract, node)
                }
            }
        }
        natspecs
    }

    /// Returns a string describing the natspec
    /// context, for debugging purposes 🐞
    /// i.e. `dir/TestContract.t.sol:FuzzContract:test_myFunction:10:12:111`
    pub fn debug_context(&self) -> String {
        format!("{}:{}:{}", self.contract, self.function, self.line)
    }

    /// Returns a list of configuration lines that match the current profile
    pub fn current_profile_configs(&self) -> Vec<String> {
        let prefix: &str = INLINE_CONFIG_PREFIX_SELECTED_PROFILE.as_ref();
        self.config_lines_with_prefix(prefix)
    }

    /// Returns a list of configuration lines that match a specific string prefix
    pub fn config_lines_with_prefix<S: Into<String>>(&self, prefix: S) -> Vec<String> {
        let prefix: String = prefix.into();
        self.config_lines().into_iter().filter(|l| l.starts_with(&prefix)).collect()
    }

    /// Returns a list of all the configuration lines available in the natspec
    pub fn config_lines(&self) -> Vec<String> {
        self.docs
            .split('\n')
            .map(remove_whitespaces)
            .filter(|line| line.contains(INLINE_CONFIG_PREFIX))
            .collect::<Vec<String>>()
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
/// If a natspec is found it is added to `natspecs`
fn apply(natspecs: &mut Vec<NatSpec>, contract: &str, node: &Node) {
    for n in node.nodes.iter() {
        if let Some((function, docs, line)) = get_fn_data(n) {
            natspecs.push(NatSpec { contract: contract.into(), function, line, docs })
        }
        apply(natspecs, contract, n);
    }
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

/// Inspects Solc compiler output for documentation comments. Returns:
/// - `Some((String, String))` in case the function has natspec comments. First item is a textual
///   natspec representation, the second item is the natspec src line, in the form "raw:col:length".
/// - `None` in case the function has not natspec comments.
fn get_fn_docs(fn_data: &BTreeMap<String, Value>) -> Option<(String, String)> {
    if let Value::Object(fn_docs) = fn_data.get("documentation")? {
        if let Value::String(comment) = fn_docs.get("text")? {
            if comment.contains(INLINE_CONFIG_PREFIX) {
                let src_line = fn_docs
                    .get("src")
                    .map(Value::to_string)
                    .unwrap_or(String::from("<no-src-line-available>"));
                return Some((comment.into(), src_line))
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::NatSpec;

    #[test]
    fn config_lines() {
        let natspec = natspec();
        let config_lines = natspec.config_lines();
        assert_eq!(
            config_lines,
            vec![
                "forge-config:default.fuzz.runs=600".to_string(),
                "forge-config:ci.fuzz.runs=500".to_string(),
                "forge-config:default.invariant.runs=1".to_string()
            ]
        )
    }

    #[test]
    fn current_profile_configs() {
        let natspec = natspec();
        let config_lines = natspec.current_profile_configs();

        assert_eq!(
            config_lines,
            vec![
                "forge-config:default.fuzz.runs=600".to_string(),
                "forge-config:default.invariant.runs=1".to_string()
            ]
        );
    }

    #[test]
    fn config_lines_with_prefix() {
        use super::INLINE_CONFIG_PREFIX;
        let natspec = natspec();
        let prefix = format!("{INLINE_CONFIG_PREFIX}:default");
        let config_lines = natspec.config_lines_with_prefix(prefix);
        assert_eq!(
            config_lines,
            vec![
                "forge-config:default.fuzz.runs=600".to_string(),
                "forge-config:default.invariant.runs=1".to_string()
            ]
        )
    }

    fn natspec() -> NatSpec {
        let conf = r#"
        forge-config: default.fuzz.runs = 600 
        forge-config: ci.fuzz.runs = 500 
        ========= SOME NOISY TEXT =============
         䩹𧀫Jx닧Ʀ̳盅K擷􅟽Ɂw첊}ꏻk86ᖪk-檻ܴ렝[ǲ𐤬oᘓƤ
        ꣖ۻ%Ƅ㪕ς:(饁΍av/烲ڻ̛߉橞㗡𥺃̹M봓䀖ؿ̄󵼁)𯖛d􂽰񮍃
        ϊ&»ϿЏ񊈞2򕄬񠪁鞷砕eߥH󶑶J粊񁼯머?槿ᴴጅ𙏑ϖ뀓򨙺򷃅Ӽ츙4󍔹
        醤㭊r􎜕󷾸𶚏 ܖ̹灱녗V*竅􋹲⒪苏贗񾦼=숽ؓ򗋲бݧ󫥛𛲍ʹ園Ьi
        =======================================
        forge-config: default.invariant.runs = 1
        "#;

        NatSpec {
            contract: "dir/TestContract.t.sol:FuzzContract".to_string(),
            function: "test_myFunction".to_string(),
            line: "10:12:111".to_string(),
            docs: conf.to_string(),
        }
    }
}
