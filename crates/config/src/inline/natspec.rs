use super::{remove_whitespaces, INLINE_CONFIG_PREFIX, INLINE_CONFIG_PREFIX_SELECTED_PROFILE};
use foundry_compilers::{
    artifacts::{ast::NodeType, Node},
    ProjectCompileOutput,
};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

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
    pub fn parse(output: &ProjectCompileOutput, root: &Path) -> Vec<Self> {
        let mut natspecs: Vec<Self> = vec![];

        for (id, artifact) in output.artifact_ids() {
            let Some(ast) = &artifact.ast else { continue };
            let path = id.source.as_path();
            let path = path.strip_prefix(root).unwrap_or(path);
            // id.identifier
            let contract = format!("{}:{}", path.display(), id.name);
            let Some(node) = contract_root_node(&ast.nodes, &contract) else { continue };
            apply(&mut natspecs, &contract, node)
        }

        natspecs
    }

    /// Returns a string describing the natspec
    /// context, for debugging purposes 🐞
    /// i.e. `test/Counter.t.sol:CounterTest:testFuzz_SetNumber`
    pub fn debug_context(&self) -> String {
        format!("{}:{}", self.contract, self.function)
    }

    /// Returns a list of configuration lines that match the current profile
    pub fn current_profile_configs(&self) -> impl Iterator<Item = String> + '_ {
        self.config_lines_with_prefix(INLINE_CONFIG_PREFIX_SELECTED_PROFILE.as_str())
    }

    /// Returns a list of configuration lines that match a specific string prefix
    pub fn config_lines_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = String> + 'a {
        self.config_lines().filter(move |l| l.starts_with(prefix))
    }

    /// Returns a list of all the configuration lines available in the natspec
    pub fn config_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.docs.lines().map(remove_whitespaces).filter(|line| line.contains(INLINE_CONFIG_PREFIX))
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

/// Given a compilation output node, if it is a function definition
/// that also contains a natspec then return a tuple of:
/// - Function name
/// - Natspec text
/// - Natspec position with format "row:col:length"
///
/// Return None otherwise.
fn get_fn_data(node: &Node) -> Option<(String, String, String)> {
    if let NodeType::FunctionDefinition = node.node_type {
        let fn_data = &node.other;
        let fn_name: String = get_fn_name(fn_data)?;
        let (fn_docs, docs_src_line): (String, String) = get_fn_docs(fn_data)?;
        return Some((fn_name, fn_docs, docs_src_line))
    }

    None
}

/// Given a dictionary of function data returns the name of the function.
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
                let mut src_line = fn_docs
                    .get("src")
                    .map(|src| src.to_string())
                    .unwrap_or_else(|| String::from("<no-src-line-available>"));

                src_line.retain(|c| c != '"');
                return Some((comment.into(), src_line))
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::{inline::natspec::get_fn_docs, NatSpec};
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    #[test]
    fn config_lines() {
        let natspec = natspec();
        let config_lines = natspec.config_lines();
        assert_eq!(
            config_lines.collect::<Vec<_>>(),
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
            config_lines.collect::<Vec<_>>(),
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
        let config_lines = natspec.config_lines_with_prefix(&prefix);
        assert_eq!(
            config_lines.collect::<Vec<_>>(),
            vec![
                "forge-config:default.fuzz.runs=600".to_string(),
                "forge-config:default.invariant.runs=1".to_string()
            ]
        )
    }

    #[test]
    fn can_handle_unavailable_src_line_with_fallback() {
        let mut fn_data: BTreeMap<String, Value> = BTreeMap::new();
        let doc_withouth_src_field = json!({ "text":  "forge-config:default.fuzz.runs=600" });
        fn_data.insert("documentation".into(), doc_withouth_src_field);
        let (_, src_line) = get_fn_docs(&fn_data).expect("Some docs");
        assert_eq!(src_line, "<no-src-line-available>".to_string());
    }

    #[test]
    fn can_handle_available_src_line() {
        let mut fn_data: BTreeMap<String, Value> = BTreeMap::new();
        let doc_withouth_src_field =
            json!({ "text":  "forge-config:default.fuzz.runs=600", "src": "73:21:12" });
        fn_data.insert("documentation".into(), doc_withouth_src_field);
        let (_, src_line) = get_fn_docs(&fn_data).expect("Some docs");
        assert_eq!(src_line, "73:21:12".to_string());
    }

    fn natspec() -> NatSpec {
        let conf = r"
        forge-config: default.fuzz.runs = 600 
        forge-config: ci.fuzz.runs = 500 
        ========= SOME NOISY TEXT =============
         䩹𧀫Jx닧Ʀ̳盅K擷􅟽Ɂw첊}ꏻk86ᖪk-檻ܴ렝[ǲ𐤬oᘓƤ
        ꣖ۻ%Ƅ㪕ς:(饁΍av/烲ڻ̛߉橞㗡𥺃̹M봓䀖ؿ̄󵼁)𯖛d􂽰񮍃
        ϊ&»ϿЏ񊈞2򕄬񠪁鞷砕eߥH󶑶J粊񁼯머?槿ᴴጅ𙏑ϖ뀓򨙺򷃅Ӽ츙4󍔹
        醤㭊r􎜕󷾸𶚏 ܖ̹灱녗V*竅􋹲⒪苏贗񾦼=숽ؓ򗋲бݧ󫥛𛲍ʹ園Ьi
        =======================================
        forge-config: default.invariant.runs = 1
        ";

        NatSpec {
            contract: "dir/TestContract.t.sol:FuzzContract".to_string(),
            function: "test_myFunction".to_string(),
            line: "10:12:111".to_string(),
            docs: conf.to_string(),
        }
    }
}
