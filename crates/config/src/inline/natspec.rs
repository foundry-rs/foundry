use super::{InlineConfigError, InlineConfigErrorKind, INLINE_CONFIG_PREFIX};
use figment::Profile;
use foundry_compilers::{
    artifacts::{ast::NodeType, Node},
    ProjectCompileOutput,
};
use itertools::Itertools;
use serde_json::Value;
use solar_parse::{
    ast::{
        interface::{self, Session},
        Arena, CommentKind, Item, ItemKind,
    },
    Parser,
};
use std::{collections::BTreeMap, path::Path};

/// Convenient struct to hold in-line per-test configurations
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NatSpec {
    /// The parent contract of the natspec.
    pub contract: String,
    /// The function annotated with the natspec. None if the natspec is contract-level.
    pub function: Option<String>,
    /// The line the natspec appears, in the form `row:col:length`, i.e. `10:21:122`.
    pub line: String,
    /// The actual natspec comment, without slashes or block punctuation.
    pub docs: String,
}

impl NatSpec {
    /// Factory function that extracts a vector of [`NatSpec`] instances from
    /// a solc compiler output. The root path is to express contract base dirs.
    /// That is essential to match per-test configs at runtime.
    pub fn parse(output: &ProjectCompileOutput, root: &Path) -> Vec<Self> {
        let mut natspecs: Vec<Self> = vec![];

        let solc = SolcParser::new();
        let solar = SolarParser::new();
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
                    solar.parse(&mut natspecs, &src, &contract, contract_name);
                }
            }
        }

        natspecs
    }

    /// Checks if all configuration lines use a valid profile.
    ///
    /// i.e. Given available profiles
    /// ```rust
    /// let _profiles = vec!["ci", "default"];
    /// ```
    /// A configuration like `forge-config: ciii.invariant.depth = 1` would result
    /// in an error.
    pub fn validate_profiles(&self, profiles: &[Profile]) -> eyre::Result<()> {
        for config in self.config_values() {
            if !profiles.iter().any(|p| {
                config
                    .strip_prefix(p.as_str().as_str())
                    .is_some_and(|rest| rest.trim_start().starts_with('.'))
            }) {
                Err(InlineConfigError {
                    location: self.location_string(),
                    kind: InlineConfigErrorKind::InvalidProfile(
                        config.to_string(),
                        profiles.iter().format(", ").to_string(),
                    ),
                })?
            }
        }
        Ok(())
    }

    /// Returns the path of the contract.
    pub fn path(&self) -> &str {
        match self.contract.split_once(':') {
            Some((path, _)) => path,
            None => self.contract.as_str(),
        }
    }

    /// Returns the location of the natspec as a string.
    pub fn location_string(&self) -> String {
        format!("{}:{}", self.path(), self.line)
    }

    /// Returns a list of all the configuration values available in the natspec.
    pub fn config_values(&self) -> impl Iterator<Item = &str> {
        self.docs.lines().filter_map(|line| {
            line.find(INLINE_CONFIG_PREFIX)
                .map(|idx| line[idx + INLINE_CONFIG_PREFIX.len()..].trim())
        })
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

struct SolarParser {
    _private: (),
}

impl SolarParser {
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

        let mut handle_docs = |item: &Item<'_>| {
            if item.docs.is_empty() {
                return;
            }
            let lines = item
                .docs
                .iter()
                .filter_map(|d| {
                    let s = d.symbol.as_str();
                    if !s.contains(INLINE_CONFIG_PREFIX) {
                        return None
                    }
                    match d.kind {
                        CommentKind::Line => Some(s.trim().to_string()),
                        CommentKind::Block => Some(
                            s.lines()
                                .filter(|line| line.contains(INLINE_CONFIG_PREFIX))
                                .map(|line| line.trim_start().trim_start_matches('*').trim())
                                .collect::<Vec<_>>()
                                .join("\n"),
                        ),
                    }
                })
                .join("\n");
            if lines.is_empty() {
                return;
            }
            let span =
                item.docs.iter().map(|doc| doc.span).reduce(|a, b| a.to(b)).unwrap_or_default();
            natspecs.push(NatSpec {
                contract: contract_id.to_string(),
                function: if let ItemKind::Function(f) = &item.kind {
                    Some(
                        f.header
                            .name
                            .map(|sym| sym.to_string())
                            .unwrap_or_else(|| f.kind.to_string()),
                    )
                } else {
                    None
                },
                line: format!("{}:{}:0", span.lo().0, span.hi().0),
                docs: lines,
            });
        };

        let sess = Session::builder()
            .with_silent_emitter(Some("Inline config parsing failed".to_string()))
            .build();
        let _ = sess.enter(|| -> interface::Result<()> {
            let arena = Arena::new();

            let mut parser = Parser::from_source_code(
                &sess,
                &arena,
                interface::source_map::FileName::Custom(contract_id.to_string()),
                src.to_string(),
            )?;

            let source_unit = parser.parse_file().map_err(|e| e.emit())?;

            for item in source_unit.items.iter() {
                let ItemKind::Contract(c) = &item.kind else { continue };
                if c.name.as_str() != contract_name {
                    continue;
                }

                // Handle contract level doc comments.
                handle_docs(item);

                // Handle function level doc comments.
                for item in c.body.iter() {
                    let ItemKind::Function(_) = &item.kind else { continue };
                    handle_docs(item);
                }
            }

            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn can_reject_invalid_profiles() {
        let profiles = ["ci".into(), "default".into()];
        let natspec = NatSpec {
            contract: Default::default(),
            function: Default::default(),
            line: Default::default(),
            docs: r"
            forge-config: ciii.invariant.depth = 1
            forge-config: default.invariant.depth = 1
            "
            .into(),
        };

        let result = natspec.validate_profiles(&profiles);
        assert!(result.is_err());
    }

    #[test]
    fn can_accept_valid_profiles() {
        let profiles = ["ci".into(), "default".into()];
        let natspec = NatSpec {
            contract: Default::default(),
            function: Default::default(),
            line: Default::default(),
            docs: r"
            forge-config: ci.invariant.depth = 1
            forge-config: default.invariant.depth = 1
            "
            .into(),
        };

        let result = natspec.validate_profiles(&profiles);
        assert!(result.is_ok());
    }

    #[test]
    fn parse_solar() {
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
        let id = || "path.sol:C".to_string();
        let solar_parser = SolarParser::new();
        solar_parser.parse(&mut natspecs, src, &id(), "C");
        assert_eq!(
            natspecs,
            [
                // f1
                NatSpec {
                    contract: id(),
                    function: Some("f1".to_string()),
                    line: "14:134:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 600\nforge-config: default.fuzz.runs = 601".to_string(),
                },
                // f2
                NatSpec {
                    contract: id(),
                    function: Some("f2".to_string()),
                    line: "164:208:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 700".to_string(),
                },
                // f3
                NatSpec {
                    contract: id(),
                    function: Some("f3".to_string()),
                    line: "226:270:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 800".to_string(),
                },
                // f4
                NatSpec {
                    contract: id(),
                    function: Some("f4".to_string()),
                    line: "289:391:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 1024\nforge-config: default.fuzz.max-test-rejects = 500".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_solar_2() {
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
        let solar = SolarParser::new();
        let id = || "inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string();
        solar.parse(&mut natspecs, src, &id(), "FuzzInlineConf");
        assert_eq!(
            natspecs,
            [
                NatSpec {
                    contract: id(),
                    function: Some("testInlineConfFuzz".to_string()),
                    line: "141:255:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 1024\nforge-config: default.fuzz.max-test-rejects = 500".to_string(),
                },
            ]
        );
    }

    #[test]
    fn config_lines() {
        let natspec = natspec();
        let config_lines = natspec.config_values();
        assert_eq!(
            config_lines.collect::<Vec<_>>(),
            [
                "default.fuzz.runs = 600".to_string(),
                "ci.fuzz.runs = 500".to_string(),
                "default.invariant.runs = 1".to_string()
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
    fn parse_solar_multiple_contracts_from_same_file() {
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
        let solar = SolarParser::new();
        let id = || "inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string();
        solar.parse(&mut natspecs, src, &id(), "FuzzInlineConf");
        assert_eq!(
            natspecs,
            [NatSpec {
                contract: id(),
                function: Some("testInlineConfFuzz1".to_string()),
                line: "142:181:0".to_string(),
                docs: "forge-config: default.fuzz.runs = 1".to_string(),
            },]
        );

        let mut natspecs = vec![];
        let id = || "inline/FuzzInlineConf2.t.sol:FuzzInlineConf2".to_string();
        solar.parse(&mut natspecs, src, &id(), "FuzzInlineConf2");
        assert_eq!(
            natspecs,
            [NatSpec {
                contract: id(),
                function: Some("testInlineConfFuzz2".to_string()),
                line: "264:303:0".to_string(),
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
        let solar = SolarParser::new();
        let id = || "inline/FuzzInlineConf.t.sol:FuzzInlineConf".to_string();
        solar.parse(&mut natspecs, src, &id(), "FuzzInlineConf");
        assert_eq!(
            natspecs,
            [
                NatSpec {
                    contract: id(),
                    function: None,
                    line: "101:140:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 1".to_string(),
                },
                NatSpec {
                    contract: id(),
                    function: Some("testInlineConfFuzz1".to_string()),
                    line: "181:220:0".to_string(),
                    docs: "forge-config: default.fuzz.runs = 3".to_string(),
                }
            ]
        );
    }
}
