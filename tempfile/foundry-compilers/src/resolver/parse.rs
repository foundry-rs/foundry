use foundry_compilers_core::utils;
use semver::VersionReq;
use solar_parse::{ast, interface::sym};
use std::{
    ops::Range,
    path::{Path, PathBuf},
};

/// Represents various information about a Solidity file.
#[derive(Clone, Debug)]
pub struct SolData {
    pub license: Option<Spanned<String>>,
    pub version: Option<Spanned<String>>,
    pub experimental: Option<Spanned<String>>,
    pub imports: Vec<Spanned<SolImport>>,
    pub version_req: Option<VersionReq>,
    pub libraries: Vec<SolLibrary>,
    pub contract_names: Vec<String>,
    pub is_yul: bool,
    pub parse_result: Result<(), String>,
}

impl SolData {
    /// Returns the result of parsing the file.
    pub fn parse_result(&self) -> crate::Result<()> {
        self.parse_result.clone().map_err(crate::SolcError::ParseError)
    }

    #[allow(dead_code)]
    pub fn fmt_version<W: std::fmt::Write>(
        &self,
        f: &mut W,
    ) -> std::result::Result<(), std::fmt::Error> {
        if let Some(version) = &self.version {
            write!(f, "({})", version.data)?;
        }
        Ok(())
    }

    /// Extracts the useful data from a solidity source
    ///
    /// This will attempt to parse the solidity AST and extract the imports and version pragma. If
    /// parsing fails, we'll fall back to extract that info via regex
    pub fn parse(content: &str, file: &Path) -> Self {
        let is_yul = file.extension().is_some_and(|ext| ext == "yul");
        let mut version = None;
        let mut experimental = None;
        let mut imports = Vec::<Spanned<SolImport>>::new();
        let mut libraries = Vec::new();
        let mut contract_names = Vec::new();
        let mut parse_result = Ok(());

        let sess = solar_parse::interface::Session::builder()
            .with_buffer_emitter(Default::default())
            .build();
        sess.enter(|| {
            let arena = ast::Arena::new();
            let filename = solar_parse::interface::source_map::FileName::Real(file.to_path_buf());
            let Ok(mut parser) =
                solar_parse::Parser::from_source_code(&sess, &arena, filename, content.to_string())
            else {
                return;
            };
            let Ok(ast) = parser.parse_file().map_err(|e| e.emit()) else { return };
            for item in ast.items {
                let loc = item.span.lo().to_usize()..item.span.hi().to_usize();
                match &item.kind {
                    ast::ItemKind::Pragma(pragma) => match &pragma.tokens {
                        ast::PragmaTokens::Version(name, req) if name.name == sym::solidity => {
                            version = Some(Spanned::new(req.to_string(), loc));
                        }
                        ast::PragmaTokens::Custom(name, value)
                            if name.as_str() == "experimental" =>
                        {
                            let value =
                                value.as_ref().map(|v| v.as_str().to_string()).unwrap_or_default();
                            experimental = Some(Spanned::new(value, loc));
                        }
                        _ => {}
                    },

                    ast::ItemKind::Import(import) => {
                        let path = import.path.value.to_string();
                        let aliases = match &import.items {
                            ast::ImportItems::Plain(None) | ast::ImportItems::Glob(None) => &[][..],
                            ast::ImportItems::Plain(Some(alias))
                            | ast::ImportItems::Glob(Some(alias)) => &[(*alias, None)][..],
                            ast::ImportItems::Aliases(aliases) => aliases,
                        };
                        let sol_import = SolImport::new(PathBuf::from(path)).set_aliases(
                            aliases
                                .iter()
                                .map(|(id, alias)| match alias {
                                    Some(al) => SolImportAlias::Contract(
                                        al.name.to_string(),
                                        id.name.to_string(),
                                    ),
                                    None => SolImportAlias::File(id.name.to_string()),
                                })
                                .collect(),
                        );
                        imports.push(Spanned::new(sol_import, loc));
                    }

                    ast::ItemKind::Contract(contract) => {
                        if contract.kind.is_library() {
                            libraries.push(SolLibrary { is_inlined: library_is_inlined(contract) });
                        }
                        contract_names.push(contract.name.to_string());
                    }

                    _ => {}
                }
            }
        });
        if let Err(e) = sess.emitted_errors().unwrap() {
            let e = e.to_string();
            trace!("failed parsing {file:?}: {e}");
            parse_result = Err(e);

            if version.is_none() {
                version = utils::capture_outer_and_inner(
                    content,
                    &utils::RE_SOL_PRAGMA_VERSION,
                    &["version"],
                )
                .first()
                .map(|(cap, name)| Spanned::new(name.as_str().to_owned(), cap.range()));
            }
            if imports.is_empty() {
                imports = capture_imports(content);
            }
            if contract_names.is_empty() {
                utils::RE_CONTRACT_NAMES.captures_iter(content).for_each(|cap| {
                    contract_names.push(cap[1].to_owned());
                });
            }
        }
        let license = content.lines().next().and_then(|line| {
            utils::capture_outer_and_inner(
                line,
                &utils::RE_SOL_SDPX_LICENSE_IDENTIFIER,
                &["license"],
            )
            .first()
            .map(|(cap, l)| Spanned::new(l.as_str().to_owned(), cap.range()))
        });
        let version_req = version.as_ref().and_then(|v| Self::parse_version_req(v.data()).ok());

        Self {
            version_req,
            version,
            experimental,
            imports,
            license,
            libraries,
            contract_names,
            is_yul,
            parse_result,
        }
    }

    /// Returns the corresponding SemVer version requirement for the solidity version.
    ///
    /// Note: This is a workaround for the fact that `VersionReq::parse` does not support whitespace
    /// separators and requires comma separated operators. See [VersionReq].
    pub fn parse_version_req(version: &str) -> Result<VersionReq, semver::Error> {
        let version = version.replace(' ', ",");

        // Somehow, Solidity semver without an operator is considered to be "exact",
        // but lack of operator automatically marks the operator as Caret, so we need
        // to manually patch it? :shrug:
        let exact = !matches!(&version[0..1], "*" | "^" | "=" | ">" | "<" | "~");
        let mut version = VersionReq::parse(&version)?;
        if exact {
            version.comparators[0].op = semver::Op::Exact;
        }

        Ok(version)
    }
}

#[derive(Clone, Debug)]
pub struct SolImport {
    path: PathBuf,
    aliases: Vec<SolImportAlias>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SolImportAlias {
    File(String),
    Contract(String, String),
}

impl SolImport {
    pub fn new(path: PathBuf) -> Self {
        Self { path, aliases: vec![] }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    pub fn aliases(&self) -> &Vec<SolImportAlias> {
        &self.aliases
    }

    fn set_aliases(mut self, aliases: Vec<SolImportAlias>) -> Self {
        self.aliases = aliases;
        self
    }
}

/// Minimal representation of a contract inside a solidity file
#[derive(Clone, Debug)]
pub struct SolLibrary {
    pub is_inlined: bool,
}

impl SolLibrary {
    /// Returns `true` if all functions of this library will be inlined.
    ///
    /// This checks if all functions are either internal or private, because internal functions can
    /// only be accessed from within the current contract or contracts deriving from it. They cannot
    /// be accessed externally. Since they are not exposed to the outside through the contract’s
    /// ABI, they can take parameters of internal types like mappings or storage references.
    ///
    /// See also <https://docs.soliditylang.org/en/latest/contracts.html#libraries>
    pub fn is_inlined(&self) -> bool {
        self.is_inlined
    }
}

/// A spanned item.
#[derive(Clone, Debug)]
pub struct Spanned<T> {
    /// The byte range of `data` in the file.
    pub span: Range<usize>,
    /// The data of the item.
    pub data: T,
}

impl<T> Spanned<T> {
    /// Creates a new data unit with the given data and location.
    pub fn new(data: T, span: Range<usize>) -> Self {
        Self { data, span }
    }

    /// Returns the underlying data.
    pub fn data(&self) -> &T {
        &self.data
    }

    /// Returns the location.
    pub fn span(&self) -> Range<usize> {
        self.span.clone()
    }

    /// Returns the location adjusted by an offset.
    ///
    /// Used to determine new position of the unit within the file after content manipulation.
    pub fn loc_by_offset(&self, offset: isize) -> Range<usize> {
        utils::range_by_offset(&self.span, offset)
    }
}

fn library_is_inlined(contract: &ast::ItemContract<'_>) -> bool {
    contract
        .body
        .iter()
        .filter_map(|item| match &item.kind {
            ast::ItemKind::Function(f) => Some(f),
            _ => None,
        })
        .all(|f| {
            !matches!(
                f.header.visibility,
                Some(ast::Visibility::Public | ast::Visibility::External)
            )
        })
}

/// Capture the import statement information together with aliases
pub fn capture_imports(content: &str) -> Vec<Spanned<SolImport>> {
    let mut imports = vec![];
    for cap in utils::RE_SOL_IMPORT.captures_iter(content) {
        if let Some(name_match) = ["p1", "p2", "p3", "p4"].iter().find_map(|name| cap.name(name)) {
            let statement_match = cap.get(0).unwrap();
            let mut aliases = vec![];
            for alias_cap in utils::RE_SOL_IMPORT_ALIAS.captures_iter(statement_match.as_str()) {
                if let Some(alias) = alias_cap.name("alias") {
                    let alias = alias.as_str().to_owned();
                    let import_alias = match alias_cap.name("target") {
                        Some(target) => SolImportAlias::Contract(alias, target.as_str().to_owned()),
                        None => SolImportAlias::File(alias),
                    };
                    aliases.push(import_alias);
                }
            }
            let sol_import =
                SolImport::new(PathBuf::from(name_match.as_str())).set_aliases(aliases);
            imports.push(Spanned::new(sol_import, statement_match.range()));
        }
    }
    imports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn assert_version(version_req: Option<&str>, src: &str) {
        let data = SolData::parse(src, "test.sol".as_ref());
        assert_eq!(data.version_req, version_req.map(|v| v.parse().unwrap()), "src:\n{src}");
    }

    #[track_caller]
    fn assert_contract_names(names: &[&str], src: &str) {
        let data = SolData::parse(src, "test.sol".as_ref());
        assert_eq!(data.contract_names, names, "src:\n{src}");
    }

    #[test]
    fn soldata_parsing() {
        assert_version(None, "");
        assert_version(None, "contract C { }");

        // https://github.com/foundry-rs/foundry/issues/9349
        assert_version(
            Some(">=0.4.22, <0.6"),
            r#"
pragma solidity >=0.4.22 <0.6;

contract BugReport {
    function() external payable {
        deposit();
    }
    function deposit() public payable {}
}
        "#,
        );

        assert_contract_names(
            &["A", "B69$_", "C_", "$D"],
            r#"
    contract A {}
library B69$_ {}
abstract contract C_ {} interface $D {}

uint constant x = .1e10;
uint constant y = .1 ether;
        "#,
        );
    }

    #[test]
    fn can_capture_curly_imports() {
        let content = r#"
import { T } from "../Test.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {DsTest} from "ds-test/test.sol";
"#;

        let captured_imports =
            capture_imports(content).into_iter().map(|s| s.data.path).collect::<Vec<_>>();

        let expected =
            utils::find_import_paths(content).map(|m| m.as_str().into()).collect::<Vec<PathBuf>>();

        assert_eq!(captured_imports, expected);

        assert_eq!(
            captured_imports,
            vec![
                PathBuf::from("../Test.sol"),
                "@openzeppelin/contracts/utils/ReentrancyGuard.sol".into(),
                "ds-test/test.sol".into(),
            ],
        );
    }

    #[test]
    fn cap_capture_aliases() {
        let content = r#"
import * as T from "./Test.sol";
import { DsTest as Test } from "ds-test/test.sol";
import "ds-test/test.sol" as Test;
import { FloatMath as Math, Math as FloatMath } from "./Math.sol";
"#;

        let caputred_imports =
            capture_imports(content).into_iter().map(|s| s.data.aliases).collect::<Vec<_>>();
        assert_eq!(
            caputred_imports,
            vec![
                vec![SolImportAlias::File("T".into())],
                vec![SolImportAlias::Contract("Test".into(), "DsTest".into())],
                vec![SolImportAlias::File("Test".into())],
                vec![
                    SolImportAlias::Contract("Math".into(), "FloatMath".into()),
                    SolImportAlias::Contract("FloatMath".into(), "Math".into()),
                ],
            ]
        );
    }
}
