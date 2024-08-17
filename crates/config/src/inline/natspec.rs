use super::{remove_whitespaces, INLINE_CONFIG_PREFIX, INLINE_CONFIG_PREFIX_SELECTED_PROFILE};
use foundry_compilers::{
    artifacts::{ast::NodeType, Node},
    ProjectCompileOutput,
};
use serde_json::Value;
use solang_parser::{helpers::CodeLocation, pt};
use std::{collections::BTreeMap, path::Path};

/// Convenient struct to hold in-line per-test configurations
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NatSpec {
    /// The parent contract of the natspec
    pub contract: String,
    /// The function annotated with the natspec. None if the natspec is contract-level
    pub function: Option<String>,
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

        let solc = SolcParser::new();
        let solang = SolangParser::new();
        for (id, artifact) in output.artifact_ids() {
            let abs_path = id.source.as_path();
            let path = abs_path.strip_prefix(root).unwrap_or(abs_path);
            let contract_name = id.name.split('.').next().unwrap();
            // `id.identifier` but with the stripped path.
            let contract = format!("{}:{}", path.display(), id.name);

            let mut used_solc_ast = false;
            if let Some(ast) = &artifact.ast {
                if let Some(node) = solc.contract_root_node(&ast.nodes, &contract) {
                    solc.parse(&mut natspecs, &contract, node, true);
                    used_solc_ast = true;
                }
            }

            if !used_solc_ast {
                if let Ok(src) = std::fs::read_to_string(abs_path) {
                    solang.parse(&mut natspecs, &src, &contract, contract_name);
                }
            }
        }

        natspecs
    }

    /// Returns a string describing the natspec
    /// context, for debugging purposes 🐞
    /// i.e. `test/Counter.t.sol:CounterTest:testFuzz_SetNumber`
    pub fn debug_context(&self) -> String {
        format!("{}:{}", self.contract, self.function.as_deref().unwrap_or_default())
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
        self.docs.lines().filter(|line| line.contains(INLINE_CONFIG_PREFIX)).map(remove_whitespaces)
    }
}

struct SolcParser {
    _private: (),
}

impl SolcParser {
    fn new() -> Self {
        Self { _private: () }
    }

    /// Given a list of nodes, find a "ContractDefinition" node that matches
    /// the provided contract_id.
    fn contract_root_node<'a>(&self, nodes: &'a [Node], contract_id: &str) -> Option<&'a Node> {
        for n in nodes.iter() {
            if n.node_type == NodeType::ContractDefinition {
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
    fn parse(&self, natspecs: &mut Vec<NatSpec>, contract: &str, node: &Node, root: bool) {
        // If we're at the root contract definition node, try parsing contract-level natspec
        if root {
            if let Some((docs, line)) = self.get_node_docs(&node.other) {
                natspecs.push(NatSpec { contract: contract.into(), function: None, docs, line })
            }
        }
        for n in node.nodes.iter() {
            if let Some((function, docs, line)) = self.get_fn_data(n) {
                natspecs.push(NatSpec {
                    contract: contract.into(),
                    function: Some(function),
                    line,
                    docs,
                })
            }
            self.parse(natspecs, contract, n, false);
        }
    }

    /// Given a compilation output node, if it is a function definition
    /// that also contains a natspec then return a tuple of:
    /// - Function name
    /// - Natspec text
    /// - Natspec position with format "row:col:length"
    ///
    /// Return None otherwise.
    fn get_fn_data(&self, node: &Node) -> Option<(String, String, String)> {
        if node.node_type == NodeType::FunctionDefinition {
            let fn_data = &node.other;
            let fn_name: String = self.get_fn_name(fn_data)?;
            let (fn_docs, docs_src_line) = self.get_node_docs(fn_data)?;
            return Some((fn_name, fn_docs, docs_src_line))
        }

        None
    }

    /// Given a dictionary of function data returns the name of the function.
    fn get_fn_name(&self, fn_data: &BTreeMap<String, Value>) -> Option<String> {
        match fn_data.get("name")? {
            Value::String(fn_name) => Some(fn_name.into()),
            _ => None,
        }
    }

    /// Inspects Solc compiler output for documentation comments. Returns:
    /// - `Some((String, String))` in case the function has natspec comments. First item is a
    ///   textual natspec representation, the second item is the natspec src line, in the form
    ///   "raw:col:length".
    /// - `None` in case the function has not natspec comments.
    fn get_node_docs(&self, data: &BTreeMap<String, Value>) -> Option<(String, String)> {
        if let Value::Object(fn_docs) = data.get("documentation")? {
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
}

struct SolangParser {
    _private: (),
}

impl SolangParser {
    fn new() -> Self {
        Self { _private: () }
    }

    fn parse(
        &self,
        natspecs: &mut Vec<NatSpec>,
        src: &str,
        contract_id: &str,
        contract_name: &str,
    ) {
        // Fast path to avoid parsing the file.
        if !src.contains(INLINE_CONFIG_PREFIX) {
            return;
        }

        let Ok((pt, comments)) = solang_parser::parse(src, 0) else { return };

        // Collects natspects from the given range.
        let mut handle_docs = |contract: &str, func: Option<&str>, start, end| {
            let docs = solang_parser::doccomment::parse_doccomments(&comments, start, end);
            natspecs.extend(
                docs.into_iter()
                    .flat_map(|doc| doc.into_comments())
                    .filter(|doc| doc.value.contains(INLINE_CONFIG_PREFIX))
                    .map(|doc| NatSpec {
                        // not possible to obtain correct value due to solang-parser bug
                        // https://github.com/hyperledger/solang/issues/1658
                        line: "0:0:0".to_string(),
                        contract: contract.to_string(),
                        function: func.map(|f| f.to_string()),
                        docs: doc.value,
                    }),
            );
        };

        let mut prev_item_end = 0;
        for item in &pt.0 {
            let pt::SourceUnitPart::ContractDefinition(c) = item else {
                prev_item_end = item.loc().end();
                continue
            };
            let Some(id) = c.name.as_ref() else {
                prev_item_end = item.loc().end();
                continue
            };
            if id.name != contract_name {
                prev_item_end = item.loc().end();
                continue
            };

            // Handle doc comments in between the previous contract and the current one.
            handle_docs(contract_id, None, prev_item_end, item.loc().start());

            let mut prev_end = c.loc.start();
            for part in &c.parts {
                let pt::ContractPart::FunctionDefinition(f) = part else { continue };
                let start = f.loc.start();
                // Handle doc comments in between the previous function and the current one.
                if let Some(name) = &f.name {
                    handle_docs(contract_id, Some(name.name.as_str()), prev_end, start);
                }
                prev_end = f.loc.end();
            }

            prev_item_end = item.loc().end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_solang() {
        let src = "
contract C { /// forge-config: default.fuzz.runs = 600

\t\t\t\t                                /// forge-config: default.fuzz.runs = 601

    function f1() {}
       /** forge-config: default.fuzz.runs = 700 */
function f2() {} /** forge-config: default.fuzz.runs = 800 */ function f3() {}

/**
 * forge-config: default.fuzz.runs = 1024
 * forge-config: default.fuzz.max-test-rejects = 500
 */
    function f4() {}
}
";
        let mut natspecs = vec![];
        let solang = SolangParser::new();
        let id = || "path.sol:C".to_string();
        let default_line = || "0:0:0".to_string();
        solang.parse(&mut natspecs, src, &id(), "C");
        assert_eq!(
            natspecs,
            [
                // f1
                NatSpec {
                    contract: id(),
                    function: Some("f1".to_string()),
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 600\nforge-config: default.fuzz.runs = 601".to_string(),
                },
                // f2
                NatSpec {
                    contract: id(),
                    function: Some("f2".to_string()),
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 700".to_string(),
                },
                // f3
                NatSpec {
                    contract: id(),
                    function: Some("f3".to_string()),
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 800".to_string(),
                },
                // f4
                NatSpec {
                    contract: id(),
                    function: Some("f4".to_string()),
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 1024\nforge-config: default.fuzz.max-test-rejects = 500".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_solang_2() {
        let src = r#"
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract FuzzInlineConf is DSTest {
    /**
     * forge-config: default.fuzz.runs = 1024
     * forge-config: default.fuzz.max-test-rejects = 500
     */
    function testInlineConfFuzz(uint8 x) public {
        require(true, "this is not going to revert");
    }
}
        "#;
        let mut natspecs = vec![];
        let solang = SolangParser::new();
        let id = || "inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string();
        let default_line = || "0:0:0".to_string();
        solang.parse(&mut natspecs, src, &id(), "FuzzInlineConf");
        assert_eq!(
            natspecs,
            [
                NatSpec {
                    contract: id(),
                    function: Some("testInlineConfFuzz".to_string()),
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 1024\nforge-config: default.fuzz.max-test-rejects = 500".to_string(),
                },
            ]
        );
    }

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
        let doc_without_src_field = json!({ "text":  "forge-config:default.fuzz.runs=600" });
        fn_data.insert("documentation".into(), doc_without_src_field);
        let (_, src_line) = SolcParser::new().get_node_docs(&fn_data).expect("Some docs");
        assert_eq!(src_line, "<no-src-line-available>".to_string());
    }

    #[test]
    fn can_handle_available_src_line() {
        let mut fn_data: BTreeMap<String, Value> = BTreeMap::new();
        let doc_without_src_field =
            json!({ "text":  "forge-config:default.fuzz.runs=600", "src": "73:21:12" });
        fn_data.insert("documentation".into(), doc_without_src_field);
        let (_, src_line) = SolcParser::new().get_node_docs(&fn_data).expect("Some docs");
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
            function: Some("test_myFunction".to_string()),
            line: "10:12:111".to_string(),
            docs: conf.to_string(),
        }
    }

    #[test]
    fn parse_solang_multiple_contracts_from_same_file() {
        let src = r#"
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.0;

import "ds-test/test.sol";

contract FuzzInlineConf is DSTest {
     /// forge-config: default.fuzz.runs = 1
    function testInlineConfFuzz1() {}
}

contract FuzzInlineConf2 is DSTest {
    /// forge-config: default.fuzz.runs = 2
    function testInlineConfFuzz2() {}
}
        "#;
        let mut natspecs = vec![];
        let solang = SolangParser::new();
        let id = || "inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string();
        let default_line = || "0:0:0".to_string();
        solang.parse(&mut natspecs, src, &id(), "FuzzInlineConf");
        assert_eq!(
            natspecs,
            [NatSpec {
                contract: id(),
                function: Some("testInlineConfFuzz1".to_string()),
                line: default_line(),
                docs: "forge-config: default.fuzz.runs = 1".to_string(),
            },]
        );

        let mut natspecs = vec![];
        let id = || "inline/FuzzInlineConf2.t.sol:FuzzInlineConf2".to_string();
        solang.parse(&mut natspecs, src, &id(), "FuzzInlineConf2");
        assert_eq!(
            natspecs,
            [NatSpec {
                contract: id(),
                function: Some("testInlineConfFuzz2".to_string()),
                line: default_line(),
                // should not get config from previous contract
                docs: "forge-config: default.fuzz.runs = 2".to_string(),
            },]
        );
    }

    #[test]
    fn parse_contract_level_config() {
        let src = r#"
// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.0;

import "ds-test/test.sol";

/// forge-config: default.fuzz.runs = 1
contract FuzzInlineConf is DSTest {
    /// forge-config: default.fuzz.runs = 3
    function testInlineConfFuzz1() {}

    function testInlineConfFuzz2() {}
}"#;
        let mut natspecs = vec![];
        let solang = SolangParser::new();
        let id = || "inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string();
        let default_line = || "0:0:0".to_string();
        solang.parse(&mut natspecs, src, &id(), "FuzzInlineConf");
        assert_eq!(
            natspecs,
            [
                NatSpec {
                    contract: id(),
                    function: None,
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 1".to_string(),
                },
                NatSpec {
                    contract: id(),
                    function: Some("testInlineConfFuzz1".to_string()),
                    line: default_line(),
                    docs: "forge-config: default.fuzz.runs = 3".to_string(),
                }
            ]
        );
    }
}
