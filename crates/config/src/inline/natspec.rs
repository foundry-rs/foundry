use super::{INLINE_CONFIG_PREFIX, InlineConfigError, InlineConfigErrorKind};
use figment::Profile;
use foundry_compilers::{
    ProjectCompileOutput,
    artifacts::{Node, ast::NodeType},
};
use itertools::Itertools;
use serde_json::Value;
use solar::{
    ast::{self, Span},
    interface::Session,
};
use std::{collections::BTreeMap, path::Path};

const HALMOS_CONFIG_PREFIX: &str = "@custom:halmos";

/// Convenient struct to hold in-line per-test configurations
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NatSpec {
    /// The parent contract of the natspec.
    pub contract: String,
    /// The function annotated with the natspec. None if the natspec is contract-level.
    pub function: Option<String>,
    /// The line the natspec begins, in the form `line:column`, i.e. `10:21`.
    pub line: String,
    /// The actual natspec comment, without slashes or block punctuation.
    pub docs: String,
}

impl NatSpec {
    /// Factory function that extracts a vector of [`NatSpec`] instances from
    /// a solc compiler output. The root path is to express contract base dirs.
    /// That is essential to match per-test configs at runtime.
    #[instrument(name = "NatSpec::parse", skip_all)]
    pub fn parse(output: &ProjectCompileOutput, root: &Path) -> Vec<Self> {
        let mut natspecs: Vec<Self> = vec![];

        let compiler = output.parser().solc().compiler();
        let solar = SolarParser::new(compiler.sess());
        let solc = SolcParser::new();
        for (id, artifact) in output.artifact_ids() {
            let path = id.source.as_path();
            let path = path.strip_prefix(root).unwrap_or(path);
            let abs_path = &*root.join(path);
            let contract_name = id.name.split('.').next().unwrap();
            // `id.identifier` but with the stripped path.
            let contract = format!("{}:{}", path.display(), id.name);

            let mut used_solar = false;
            compiler.enter_sequential(|compiler| {
                if let Some((_, source)) = compiler.gcx().get_ast_source(abs_path)
                    && let Some(ast) = &source.ast
                {
                    solar.parse_ast(&mut natspecs, ast, &contract, contract_name);
                    used_solar = true;
                }
            });

            if !used_solar {
                warn!(?abs_path, %contract, "could not parse natspec with solar");
            }

            let used_solc = if !used_solar
                && let Some(ast) = &artifact.ast
                && let Some(node) = solc.contract_root_node(&ast.nodes, &contract)
            {
                solc.parse(&mut natspecs, &contract, node, true);
                true
            } else {
                false
            };

            if !used_solar && !used_solc {
                warn!(?abs_path, %contract, "could not parse natspec");
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

    /// Returns Foundry inline config values translated from legacy Halmos annotations.
    ///
    /// Native `forge-config:` annotations remain preferred. This compatibility path only
    /// translates the Halmos flags that map cleanly onto Foundry symbolic config.
    pub fn halmos_config_values(&self) -> Result<Vec<String>, InlineConfigError> {
        let mut values = Vec::new();
        for line in self.docs.lines() {
            let Some(idx) = line.find(HALMOS_CONFIG_PREFIX) else { continue };
            let args = line[idx + HALMOS_CONFIG_PREFIX.len()..].trim();
            translate_halmos_config(args, &mut values).map_err(|message| InlineConfigError {
                location: self.location_string(),
                kind: InlineConfigErrorKind::InvalidHalmosConfig(message),
            })?;
        }
        Ok(values)
    }
}

fn translate_halmos_config(args: &str, values: &mut Vec<String>) -> Result<(), String> {
    let tokens = split_halmos_args(args)?;
    let mut idx = 0;
    while idx < tokens.len() {
        let token = &tokens[idx];
        if let Some((arg, value)) = halmos_arg(token) {
            let value = if let Some(value) = value {
                value
            } else {
                idx += 1;
                tokens.get(idx).ok_or_else(|| format!("missing value for {}", arg.flag))?
            };
            emit_halmos_arg(values, arg, value)?;
        } else if token.starts_with("--")
            && tokens.get(idx + 1).is_some_and(|next| !next.starts_with("--"))
        {
            idx += 1;
        }
        idx += 1;
    }
    Ok(())
}

fn split_halmos_args(args: &str) -> Result<Vec<String>, String> {
    shlex::split(args).ok_or_else(|| "invalid shell quoting in @custom:halmos config".to_string())
}

#[derive(Clone, Copy)]
struct HalmosArg {
    flag: &'static str,
    parser: HalmosArgParser,
    foundry_field: &'static str,
}

#[derive(Clone, Copy)]
enum HalmosArgParser {
    ArrayLengths,
    Lengths,
    U32,
    String,
}

const HALMOS_ARGS: &[HalmosArg] = &[
    HalmosArg {
        flag: "--array-lengths",
        parser: HalmosArgParser::ArrayLengths,
        foundry_field: "array_lengths",
    },
    HalmosArg {
        flag: "--default-array-lengths",
        parser: HalmosArgParser::Lengths,
        foundry_field: "default_array_lengths",
    },
    HalmosArg {
        flag: "--default-bytes-lengths",
        parser: HalmosArgParser::Lengths,
        foundry_field: "default_bytes_lengths",
    },
    HalmosArg { flag: "--loop", parser: HalmosArgParser::U32, foundry_field: "loop" },
    HalmosArg {
        flag: "--invariant-depth",
        parser: HalmosArgParser::U32,
        foundry_field: "invariant_depth",
    },
    HalmosArg { flag: "--width", parser: HalmosArgParser::U32, foundry_field: "width" },
    HalmosArg { flag: "--depth", parser: HalmosArgParser::U32, foundry_field: "depth" },
    HalmosArg { flag: "--solver-timeout", parser: HalmosArgParser::U32, foundry_field: "timeout" },
    HalmosArg {
        flag: "--solver-timeout-branching",
        parser: HalmosArgParser::U32,
        foundry_field: "timeout",
    },
    HalmosArg {
        flag: "--solver-timeout-assertion",
        parser: HalmosArgParser::U32,
        foundry_field: "timeout",
    },
    HalmosArg { flag: "--solver", parser: HalmosArgParser::String, foundry_field: "solver" },
    HalmosArg {
        flag: "--solver-command",
        parser: HalmosArgParser::String,
        foundry_field: "solver_command",
    },
];

fn halmos_arg(token: &str) -> Option<(HalmosArg, Option<&str>)> {
    let (flag, value) =
        token.split_once('=').map_or((token, None), |(flag, value)| (flag, Some(value)));
    HALMOS_ARGS.iter().copied().find(|arg| arg.flag == flag).map(|arg| (arg, value))
}

fn emit_halmos_arg(values: &mut Vec<String>, arg: HalmosArg, value: &str) -> Result<(), String> {
    match arg.parser {
        HalmosArgParser::ArrayLengths => push_halmos_array_lengths(values, value),
        HalmosArgParser::Lengths => push_halmos_lengths(values, arg.foundry_field, value, arg.flag),
        HalmosArgParser::U32 => {
            push_halmos_u32(values, arg.foundry_field, parse_halmos_u32(value, arg.flag)?);
            Ok(())
        }
        HalmosArgParser::String => {
            push_halmos_string(values, arg.foundry_field, value);
            Ok(())
        }
    }
}

fn push_halmos_array_lengths(values: &mut Vec<String>, value: &str) -> Result<(), String> {
    match parse_halmos_array_lengths(value)? {
        HalmosArrayLengths::Positional(lengths) => {
            values.push(format!(
                "default.symbolic.array_lengths = [{}]",
                lengths.iter().format(", ")
            ));
        }
        HalmosArrayLengths::Named(lengths) => {
            values.push(format!(
                "default.symbolic.dynamic_lengths = {{ {} }}",
                lengths
                    .iter()
                    .map(|(name, lengths)| format!("{name} = [{}]", lengths.iter().format(", ")))
                    .format(", ")
            ));
        }
    }
    Ok(())
}

fn push_halmos_lengths(
    values: &mut Vec<String>,
    field: &str,
    value: &str,
    flag: &str,
) -> Result<(), String> {
    values.push(format!(
        "default.symbolic.{field} = [{}]",
        parse_halmos_lengths(value, flag)?.iter().format(", ")
    ));
    Ok(())
}

fn push_halmos_u32(values: &mut Vec<String>, field: &str, value: u32) {
    values.push(format!("default.symbolic.{field} = {value}"));
}

fn push_halmos_string(values: &mut Vec<String>, field: &str, value: &str) {
    values.push(format!("default.symbolic.{field} = {}", toml_string(value)));
}

fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn parse_halmos_u32(value: &str, flag: &str) -> Result<u32, String> {
    value.parse::<u32>().map_err(|_| format!("invalid value `{value}` for {flag}"))
}

enum HalmosArrayLengths {
    Positional(Vec<u32>),
    Named(BTreeMap<String, Vec<u32>>),
}

fn parse_halmos_array_lengths(value: &str) -> Result<HalmosArrayLengths, String> {
    if value.contains('=') {
        let mut named = BTreeMap::new();
        for entry in split_halmos_lengths_entries(value)? {
            let Some((name, lengths)) = entry.split_once('=') else {
                return Err(format!(
                    "mixed named and positional lengths in --array-lengths `{value}`"
                ));
            };
            let name = name.trim();
            if name.is_empty() {
                return Err(format!("missing name in --array-lengths `{value}`"));
            }
            named.insert(
                name.to_string(),
                parse_halmos_length_set(lengths.trim(), "--array-lengths")?,
            );
        }
        if named.is_empty() {
            return Err("missing value for --array-lengths".to_string());
        }
        Ok(HalmosArrayLengths::Named(named))
    } else {
        Ok(HalmosArrayLengths::Positional(parse_halmos_lengths(value, "--array-lengths")?))
    }
}

fn parse_halmos_lengths(value: &str, flag: &str) -> Result<Vec<u32>, String> {
    parse_halmos_length_set(value, flag)
}

fn parse_halmos_length_set(value: &str, flag: &str) -> Result<Vec<u32>, String> {
    let value = value.trim();
    let value = value.strip_prefix('{').and_then(|value| value.strip_suffix('}')).unwrap_or(value);
    let mut lengths = Vec::new();
    for length in value.split(',') {
        let length = length.trim();
        if length.is_empty() {
            return Err(format!("invalid empty length in {flag} `{value}`"));
        }
        let length = length
            .parse::<u32>()
            .map_err(|_| format!("invalid length `{length}` in {flag} `{value}`"))?;
        lengths.push(length);
    }
    if lengths.is_empty() {
        return Err(format!("missing value for {flag}"));
    }
    Ok(lengths)
}

fn split_halmos_lengths_entries(value: &str) -> Result<Vec<&str>, String> {
    let mut entries = Vec::new();
    let mut start = 0usize;
    let mut brace_depth = 0u8;
    for (idx, ch) in value.char_indices() {
        match ch {
            '{' => brace_depth = brace_depth.saturating_add(1),
            '}' => {
                brace_depth = brace_depth
                    .checked_sub(1)
                    .ok_or_else(|| format!("unmatched `}}` in --array-lengths `{value}`"))?;
            }
            ',' if brace_depth == 0 => {
                entries.push(value[start..idx].trim());
                start = idx + 1;
            }
            _ => {}
        }
    }
    if brace_depth != 0 {
        return Err(format!("unmatched `{{` in --array-lengths `{value}`"));
    }
    entries.push(value[start..].trim());
    Ok(entries)
}

struct SolcParser {
    _private: (),
}

impl SolcParser {
    const fn new() -> Self {
        Self { _private: () }
    }

    /// Given a list of nodes, find a "ContractDefinition" node that matches
    /// the provided contract_id.
    fn contract_root_node<'a>(&self, nodes: &'a [Node], contract_id: &str) -> Option<&'a Node> {
        for n in nodes {
            if n.node_type == NodeType::ContractDefinition {
                let contract_data = &n.other;
                if let Value::String(contract_name) = contract_data.get("name")?
                    && contract_id.ends_with(contract_name)
                {
                    return Some(n);
                }
            }
        }
        None
    }

    /// Implements a DFS over a compiler output node and its children.
    /// If a natspec is found it is added to `natspecs`
    fn parse(&self, natspecs: &mut Vec<NatSpec>, contract: &str, node: &Node, root: bool) {
        // If we're at the root contract definition node, try parsing contract-level natspec
        if root && let Some((docs, line)) = self.get_node_docs(&node.other) {
            natspecs.push(NatSpec { contract: contract.into(), function: None, docs, line })
        }
        for n in &node.nodes {
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
            return Some((fn_name, fn_docs, docs_src_line));
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
        if let Value::Object(fn_docs) = data.get("documentation")?
            && let Value::String(comment) = fn_docs.get("text")?
            && contains_inline_config(comment)
        {
            let mut src_line = fn_docs
                .get("src")
                .map(|src| src.to_string())
                .unwrap_or_else(|| String::from("<no-src-line-available>"));

            src_line.retain(|c| c != '"');
            return Some((comment.into(), src_line));
        }
        None
    }
}

struct SolarParser<'a> {
    sess: &'a Session,
}

impl<'a> SolarParser<'a> {
    const fn new(sess: &'a Session) -> Self {
        Self { sess }
    }

    fn parse_ast(
        &self,
        natspecs: &mut Vec<NatSpec>,
        source_unit: &ast::SourceUnit<'_>,
        contract_id: &str,
        contract_name: &str,
    ) {
        let mut handle_docs = |item: &ast::Item<'_>| {
            if item.docs.is_empty() {
                return;
            }
            let mut span = Span::DUMMY;
            let lines = item
                .docs
                .iter()
                .filter_map(|d| {
                    let s = d.symbol.as_str();
                    if !contains_inline_config(s) {
                        return None;
                    }
                    span = if span.is_dummy() { d.span } else { span.to(d.span) };
                    match d.kind {
                        ast::CommentKind::Line => Some(s.trim().to_string()),
                        ast::CommentKind::Block => Some(
                            s.lines()
                                .filter(|line| contains_inline_config(line))
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
            natspecs.push(NatSpec {
                contract: contract_id.to_string(),
                function: if let ast::ItemKind::Function(f) = &item.kind {
                    Some(
                        f.header
                            .name
                            .map(|sym| sym.to_string())
                            .unwrap_or_else(|| f.kind.to_string()),
                    )
                } else {
                    None
                },
                line: {
                    let (_, loc) = self.sess.source_map().span_to_location_info(span);
                    format!("{}:{}", loc.lo.line, loc.lo.col.0 + 1)
                },
                docs: lines,
            });
        };

        for item in source_unit.items.iter() {
            let ast::ItemKind::Contract(c) = &item.kind else { continue };
            if c.name.as_str() != contract_name {
                continue;
            }

            // Handle contract level doc comments.
            handle_docs(item);

            // Handle function level doc comments.
            for item in c.body.iter() {
                let ast::ItemKind::Function(_) = &item.kind else { continue };
                handle_docs(item);
            }
        }
    }
}

fn contains_inline_config(s: &str) -> bool {
    s.contains(INLINE_CONFIG_PREFIX) || s.contains(HALMOS_CONFIG_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use snapbox::{assert_data_eq, str};
    use solar::parse::{
        Parser,
        ast::{Arena, interface},
    };

    fn parse(natspecs: &mut Vec<NatSpec>, src: &str, contract_id: &str, contract_name: &str) {
        // Fast path to avoid parsing the file.
        if !contains_inline_config(src) {
            return;
        }

        let sess = Session::builder()
            .with_silent_emitter(Some("Inline config parsing failed".to_string()))
            .build();
        let solar = SolarParser::new(&sess);
        let _ = sess.enter(|| -> interface::Result<()> {
            let arena = Arena::new();

            let mut parser = Parser::from_source_code(
                &sess,
                &arena,
                interface::source_map::FileName::Custom(contract_id.to_string()),
                src.to_string(),
            )?;

            let source_unit = parser.parse_file().map_err(|e| e.emit())?;

            solar.parse_ast(natspecs, &source_unit, contract_id, contract_name);

            Ok(())
        });
    }

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
        parse(&mut natspecs, src, "path.sol:C", "C");
        assert_data_eq!(
            format!("{natspecs:#?}"),
            str![[r#"
[
    NatSpec {
        contract: "path.sol:C",
        function: Some(
            "f1",
        ),
        line: "2:14",
        docs: "forge-config: default.fuzz.runs = 600/nforge-config: default.fuzz.runs = 601",
    },
    NatSpec {
        contract: "path.sol:C",
        function: Some(
            "f2",
        ),
        line: "7:8",
        docs: "forge-config: default.fuzz.runs = 700",
    },
    NatSpec {
        contract: "path.sol:C",
        function: Some(
            "f3",
        ),
        line: "8:18",
        docs: "forge-config: default.fuzz.runs = 800",
    },
    NatSpec {
        contract: "path.sol:C",
        function: Some(
            "f4",
        ),
        line: "10:1",
        docs: "forge-config: default.fuzz.runs = 1024/nforge-config: default.fuzz.max-test-rejects = 500",
    },
]
"#]]
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
        parse(&mut natspecs, src, "inline/FuzzInlineConf.t.sol:FuzzInlineConf", "FuzzInlineConf");
        assert_data_eq!(
            format!("{natspecs:#?}"),
            str![[r#"
[
    NatSpec {
        contract: "inline/FuzzInlineConf.t.sol:FuzzInlineConf",
        function: Some(
            "testInlineConfFuzz",
        ),
        line: "8:5",
        docs: "forge-config: default.fuzz.runs = 1024/nforge-config: default.fuzz.max-test-rejects = 500",
    },
]
"#]]
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
        parse(&mut natspecs, src, "inline/FuzzInlineConf.t.sol:FuzzInlineConf", "FuzzInlineConf");
        assert_data_eq!(
            format!("{natspecs:#?}"),
            str![[r#"
[
    NatSpec {
        contract: "inline/FuzzInlineConf.t.sol:FuzzInlineConf",
        function: Some(
            "testInlineConfFuzz1",
        ),
        line: "8:6",
        docs: "forge-config: default.fuzz.runs = 1",
    },
]
"#]]
        );

        let mut natspecs = vec![];
        parse(
            &mut natspecs,
            src,
            "inline/FuzzInlineConf2.t.sol:FuzzInlineConf2",
            "FuzzInlineConf2",
        );
        assert_data_eq!(
            format!("{natspecs:#?}"),
            str![[r#"
[
    NatSpec {
        contract: "inline/FuzzInlineConf2.t.sol:FuzzInlineConf2",
        function: Some(
            "testInlineConfFuzz2",
        ),
        line: "13:5",
        docs: "forge-config: default.fuzz.runs = 2",
    },
]
"#]]
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
        parse(&mut natspecs, src, "inline/FuzzInlineConf.t.sol:FuzzInlineConf", "FuzzInlineConf");
        assert_data_eq!(
            format!("{natspecs:#?}"),
            str![[r#"
[
    NatSpec {
        contract: "inline/FuzzInlineConf.t.sol:FuzzInlineConf",
        function: None,
        line: "7:1",
        docs: "forge-config: default.fuzz.runs = 1",
    },
    NatSpec {
        contract: "inline/FuzzInlineConf.t.sol:FuzzInlineConf",
        function: Some(
            "testInlineConfFuzz1",
        ),
        line: "9:5",
        docs: "forge-config: default.fuzz.runs = 3",
    },
]
"#]]
        );
    }

    #[test]
    fn translates_legacy_halmos_array_lengths() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("checkBytes".to_string()),
            line: "10:1".to_string(),
            docs: "@custom:halmos --loop 256 --array-lengths 2,4,8 --depth 100".to_string(),
        };

        assert_eq!(
            natspec.halmos_config_values().unwrap(),
            vec![
                "default.symbolic.loop = 256",
                "default.symbolic.array_lengths = [2, 4, 8]",
                "default.symbolic.depth = 100",
            ]
        );
    }

    #[test]
    fn translates_named_halmos_array_lengths() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("checkBytes".to_string()),
            line: "10:1".to_string(),
            docs: "@custom:halmos --array-lengths values={2,4},data=8".to_string(),
        };

        assert_eq!(
            natspec.halmos_config_values().unwrap(),
            vec!["default.symbolic.dynamic_lengths = { data = [8], values = [2, 4] }"]
        );
    }

    #[test]
    fn translates_halmos_default_dynamic_length_sets() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("checkBytes".to_string()),
            line: "10:1".to_string(),
            docs:
                "@custom:halmos --default-array-lengths 0,1,2 --default-bytes-lengths={0,65,1024}"
                    .to_string(),
        };

        assert_eq!(
            natspec.halmos_config_values().unwrap(),
            vec![
                "default.symbolic.default_array_lengths = [0, 1, 2]",
                "default.symbolic.default_bytes_lengths = [0, 65, 1024]",
            ]
        );
    }

    #[test]
    fn translates_legacy_halmos_width_depth_and_solver_timeout() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("invariant_state".to_string()),
            line: "10:1".to_string(),
            docs: "@custom:halmos --width=32 --depth 128 --solver-timeout-branching 5".to_string(),
        };

        assert_eq!(
            natspec.halmos_config_values().unwrap(),
            vec![
                "default.symbolic.width = 32",
                "default.symbolic.depth = 128",
                "default.symbolic.timeout = 5",
            ]
        );
    }

    #[test]
    fn translates_legacy_halmos_solver_selection() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("check_solver".to_string()),
            line: "10:1".to_string(),
            docs: "@custom:halmos --solver cvc5 --solver-command \"bitwuzla --produce-models\""
                .to_string(),
        };

        assert_eq!(
            natspec.halmos_config_values().unwrap(),
            vec![
                "default.symbolic.solver = \"cvc5\"",
                "default.symbolic.solver_command = \"bitwuzla --produce-models\"",
            ]
        );
    }

    #[test]
    fn rejects_malformed_legacy_halmos_array_lengths() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("checkBytes".to_string()),
            line: "10:1".to_string(),
            docs: "@custom:halmos --array-lengths nope".to_string(),
        };

        let err = natspec.halmos_config_values().unwrap_err();

        assert!(err.to_string().contains("invalid @custom:halmos annotation"));
        assert!(err.to_string().contains("invalid length `nope`"));
    }

    #[test]
    fn ignores_unsupported_halmos_flags_while_translating_supported_ones() {
        let natspec = NatSpec {
            contract: "dir/TestContract.t.sol:SymbolicContract".to_string(),
            function: Some("checkBytes".to_string()),
            line: "10:1".to_string(),
            docs: "@custom:halmos --unsupported value --loop 10 --another --array-lengths 2"
                .to_string(),
        };

        assert_eq!(
            natspec.halmos_config_values().unwrap(),
            vec!["default.symbolic.loop = 10", "default.symbolic.array_lengths = [2]",]
        );
    }

    #[test]
    fn parse_solar_legacy_halmos_only_config() {
        let src = r#"
contract SymbolicHalmosLengths {
    /// @custom:halmos --array-lengths 3
    function checkArray(uint256[] memory values) public pure {
        values;
    }
}
        "#;
        let mut natspecs = vec![];
        parse(
            &mut natspecs,
            src,
            "inline/SymbolicHalmosLengths.t.sol:SymbolicHalmosLengths",
            "SymbolicHalmosLengths",
        );

        assert_eq!(natspecs.len(), 1);
        assert_eq!(
            natspecs[0].halmos_config_values().unwrap(),
            vec!["default.symbolic.array_lengths = [3]"]
        );
    }

    #[test]
    fn split_halmos_args_rejects_unterminated_quote() {
        let err = split_halmos_args(r#"--width "unterm"#).unwrap_err();
        assert_eq!(err, "invalid shell quoting in @custom:halmos config");
    }
}
