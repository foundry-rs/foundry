//! ERC-7201 namespace discovery and conversion to solc-compatible storage layouts.
//!
//! Packing and type generation are delegated to Solar. Callers decide how to merge namespaces
//! with conventional storage and how to handle duplicate namespace declarations.

use alloy_primitives::{U256, keccak256};
use foundry_compilers::{artifacts::StorageLayout, error::SolcError};
use path_slash::PathExt;
use solar::{
    ast::NatSpecKind,
    sema::{
        Compiler, Gcx, hir,
        interface::{Session, config::CompilerStage},
    },
};
use std::{ops::ControlFlow, path::Path};

/// Computes ERC-7201 layouts for a contract in the resolved project.
///
/// Lowering and analysis errors are returned rather than silently emitting incomplete layouts.
pub fn erc7201_storage_layouts(
    compiler: &mut Compiler,
    target_path: &Path,
    target_name: Option<&str>,
) -> Result<Vec<StorageNamespace>, SolcError> {
    compiler.enter_mut(|compiler| {
        if !matches!(
            compiler.gcx().stage(),
            Some(CompilerStage::Lowering | CompilerStage::Analysis)
        ) && !matches!(compiler.lower_asts(), Ok(ControlFlow::Continue(())))
        {
            return Err(solar_error(compiler.sess(), "lowering"));
        }
        if compiler.sess().dcx.has_errors().is_err() {
            return Err(solar_error(compiler.sess(), "lowering"));
        }
        let gcx = compiler.gcx();
        let mut matches = gcx.hir.contract_ids().filter(|&id| {
            let contract = gcx.hir.contract(id);
            target_name.is_none_or(|name| contract.name.as_str() == name)
                && gcx
                    .hir
                    .source(contract.source)
                    .file
                    .name
                    .as_real()
                    .is_some_and(|path| path == target_path)
        });
        let target =
            matches.next().ok_or_else(|| SolcError::msg("storage layout contract not found"))?;
        if matches.next().is_some() {
            return Err(SolcError::msg("multiple contracts found; specify <path>:<contract>"));
        }
        // Conventional layouts do not need semantic analysis. Inspect raw parsed tags before
        // requesting validated NatSpec or type information.
        let bases = gcx.hir.contract(target).linearized_bases;
        let bases = if bases.is_empty() { std::slice::from_ref(&target) } else { bases };
        let annotated = bases.iter().any(|&base| {
            gcx.hir.contract(base).items.iter().any(|item| {
                item.as_struct().is_some_and(|id| {
                    let doc = gcx.hir.doc(gcx.hir.strukt(id).doc);
                    doc.ast_comments().iter().any(|doc| {
                        doc.natspec.iter().any(|tag| {
                            matches!(tag.kind, NatSpecKind::Custom { name }
                                    if name.as_str() == "storage-location")
                                && tag.content().trim().starts_with("erc7201")
                        })
                    })
                })
            })
        });
        if !annotated {
            return Ok(Vec::new());
        }
        if compiler.gcx().stage() != Some(CompilerStage::Analysis)
            && !matches!(compiler.analysis(), Ok(ControlFlow::Continue(())))
        {
            return Err(solar_error(compiler.sess(), "analysis"));
        }
        collect_storage_layouts(compiler.gcx(), target)
    })
}

/// A storage namespace declared by a struct in a contract or one of its bases.
#[derive(Clone, Debug)]
pub struct StorageNamespace {
    /// The identifier following `erc7201:` in the storage-location annotation.
    pub id: String,
    /// The fully qualified name of the declaring struct.
    pub declaration: String,
    /// The ERC-7201 root slot.
    pub root: U256,
    /// The struct's fields at absolute slots, with nested members at relative slots.
    ///
    /// Type IDs are scoped to this namespace so they cannot alias solc-generated IDs.
    /// AST IDs originate from Solar and are not solc AST IDs.
    pub layout: StorageLayout,
}

/// Computes `keccak256(abi.encode(uint256(keccak256(id)) - 1)) & ~uint256(0xff)`.
fn erc7201_root(id: &str) -> U256 {
    let inner = U256::from_be_bytes(keccak256(id.as_bytes()).0).wrapping_sub(U256::from(1));
    U256::from_be_bytes(keccak256(inner.to_be_bytes::<32>()).0) & !U256::from(0xff)
}

/// Returns namespaces declared in `contract` and its linearized bases, once per declaration.
///
/// The compiler must have completed lowering and semantic analysis. Unrelated contracts,
/// imported libraries, and file-level structs are excluded: importing a declaration does not
/// establish that the selected contract uses its storage. Unknown storage-location schemes
/// are ignored; malformed ERC-7201 annotations are errors.
fn collect_storage_layouts(
    gcx: Gcx<'_>,
    contract: hir::ContractId,
) -> Result<Vec<StorageNamespace>, SolcError> {
    if gcx.sess.dcx.has_errors().is_err() {
        return Err(solar_error(gcx.sess, "analysis"));
    }
    let bases = gcx.hir.contract(contract).linearized_bases;
    let bases = if bases.is_empty() { std::slice::from_ref(&contract) } else { bases };
    let mut namespaces = Vec::new();
    for &base in bases.iter().rev() {
        for item in gcx.hir.contract(base).items {
            if let Some(id) = item.as_struct() {
                let strukt = gcx.hir.strukt(id);
                let contract = gcx.hir.contract(base);
                let file = &gcx.hir.source(contract.source).file.name;
                let contract_name = if let Some(path) = file.as_real() {
                    let path = gcx
                        .sess
                        .opts
                        .base_path
                        .as_ref()
                        .and_then(|root| path.strip_prefix(root).ok())
                        .unwrap_or(path);
                    format!("{}:{}", path.to_slash_lossy(), contract.name)
                } else {
                    gcx.contract_fully_qualified_name(base).to_string()
                };
                let declaration = format!("{contract_name}.{}", strukt.name);
                let mut namespace = None;
                for tag in gcx.natspec_doc_comments(strukt.doc) {
                    if let NatSpecKind::Custom { name } = tag.kind
                        && name.as_str() == "storage-location"
                        && let Some(id) = parse_namespace(tag.content())?
                        && namespace.replace(id).is_some()
                    {
                        return Err(SolcError::msg(format!(
                            "multiple ERC-7201 annotations on `{declaration}`"
                        )));
                    }
                }
                if let Some(namespace) = namespace {
                    let root = erc7201_root(namespace);
                    let mut output = gcx.storage_layout_for_struct(id, root);
                    // Match solc's project-relative source names at every level of the type graph.
                    for entry in &mut output.storage {
                        entry.contract.clone_from(&contract_name);
                    }
                    if let Some(types) = &mut output.types {
                        for member in types.values_mut().flat_map(|ty| &mut ty.members) {
                            member.contract.clone_from(&contract_name);
                        }
                    }
                    let mut layout: StorageLayout =
                        serde_json::from_value(serde_json::to_value(output)?)?;
                    scope_types(&mut layout, namespace);
                    namespaces.push(StorageNamespace {
                        id: namespace.to_owned(),
                        declaration,
                        root,
                        layout,
                    });
                }
            }
        }
    }
    Ok(namespaces)
}

fn solar_error(sess: &Session, stage: &str) -> SolcError {
    let diagnostics = sess.dcx.emitted_diagnostics().map(|d| d.to_string()).unwrap_or_default();
    SolcError::msg(format!("Solar {stage} failed while inspecting ERC-7201 storage\n{diagnostics}"))
}

fn parse_namespace(annotation: &str) -> Result<Option<&str>, SolcError> {
    let annotation = annotation.trim();
    let Some(id) = annotation.strip_prefix("erc7201:") else {
        if annotation.split_whitespace().next() == Some("erc7201") {
            return Err(SolcError::msg("expected `erc7201:<namespace>` storage location"));
        }
        return Ok(None);
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(SolcError::msg(
            "ERC-7201 namespace must be nonempty and contain no whitespace",
        ));
    }
    Ok(Some(id))
}

fn scope_types(layout: &mut StorageLayout, namespace: &str) {
    let scope = |id: &str| format!("erc7201({namespace})::{id}");
    for entry in &mut layout.storage {
        entry.storage_type = scope(&entry.storage_type);
    }
    layout.types = std::mem::take(&mut layout.types)
        .into_iter()
        .map(|(id, mut ty)| {
            for reference in [&mut ty.key, &mut ty.value].into_iter().flatten() {
                *reference = scope(reference);
            }
            if let Some(serde_json::Value::String(base)) = ty.other.get_mut("base") {
                *base = scope(base);
            }
            if let Some(serde_json::Value::Array(members)) = ty.other.get_mut("members") {
                for member in members {
                    if let Some(serde_json::Value::String(id)) = member.get_mut("type") {
                        *id = scope(id);
                    }
                }
            }
            (scope(&id), ty)
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parser_at(source: &str, path: &str) -> Compiler {
        let mut compiler = Compiler::new(Session::builder().with_test_emitter().build());
        compiler.enter_mut(|compiler| {
            let file =
                compiler.sess().source_map().new_source_file(PathBuf::from(path), source).unwrap();
            let mut parser = compiler.parse();
            parser.add_file(file);
            parser.parse();
        });
        compiler
    }

    fn parser(source: &str) -> Compiler {
        parser_at(source, "test.sol")
    }

    #[test]
    fn project_relative_source_names() {
        let mut parser = parser_at(
            r#"contract C {
            struct Inner { uint256 x; }
            /// @custom:storage-location erc7201:example
            struct Data { Inner inner; }
        }"#,
            "/project/src/test.sol",
        );
        parser.sess_mut().opts.base_path = Some(PathBuf::from("/project"));
        let namespaces =
            erc7201_storage_layouts(&mut parser, Path::new("/project/src/test.sol"), Some("C"))
                .unwrap();
        let namespace = &namespaces[0];
        assert_eq!(namespace.declaration, "src/test.sol:C.Data");
        assert_eq!(namespace.layout.storage[0].contract, "src/test.sol:C");
        let ty = &namespace.layout.types[&namespace.layout.storage[0].storage_type];
        assert_eq!(ty.other["members"][0]["contract"], "src/test.sol:C");
    }

    fn layouts(source: &str, contract: &str) -> Result<Vec<StorageNamespace>, SolcError> {
        erc7201_storage_layouts(&mut parser(source), Path::new("test.sol"), Some(contract))
    }

    #[test]
    fn repeated_inspection() {
        let mut parser = parser(
            r#"contract C {
            /// @custom:storage-location erc7201:example
            struct Data { uint256 value; }
        }"#,
        );
        let first = erc7201_storage_layouts(&mut parser, Path::new("test.sol"), Some("C")).unwrap();
        let second =
            erc7201_storage_layouts(&mut parser, Path::new("test.sol"), Some("C")).unwrap();
        assert_eq!(first[0].layout, second[0].layout);
    }

    #[test]
    fn inherited_namespaces_and_type_graph() {
        let namespaces = layouts(
            r#"
            type Amount is uint128;
            contract Base {
                enum State { Off, On }
                struct Nested { uint128 x; bool y; }
                /** @custom:storage-location erc7201:example.base */
                struct Data {
                    address owner;
                    bool paused;
                    Nested nested;
                    uint128[3] fixedValues;
                    mapping(address => Nested[]) accounts;
                    bytes data;
                    string name;
                    Amount amount;
                    State state;
                    function() external callback;
                }
            }
            contract Left is Base {}
            contract Right is Base {}
            contract Derived is Left, Right {
                /// @custom:storage-location erc7201:example.derived
                struct OtherData { uint256 value; }
            }
            contract Unrelated {
                /// @custom:storage-location erc7201:example.base
                struct Data { uint256 ignored; }
            }
        "#,
            "Derived",
        )
        .unwrap();
        assert_eq!(
            namespaces.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
            ["example.base", "example.derived"]
        );
        assert_eq!(namespaces[0].declaration, "test.sol:Base.Data");
        let layout = &namespaces[0].layout;
        let root = namespaces[0].root;
        assert_eq!(
            layout
                .storage
                .iter()
                .map(|s| (s.slot.parse::<U256>().unwrap() - root, s.offset))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 20), (1, 0), (2, 0), (4, 0), (5, 0), (6, 0), (7, 0), (7, 16), (8, 0)]
                .map(|(slot, offset)| (U256::from(slot), offset))
        );
        for ty in layout.types.values() {
            for id in [&ty.key, &ty.value].into_iter().flatten() {
                assert!(layout.types.contains_key(id), "missing {id}");
            }
            if let Some(base) = ty.other.get("base") {
                assert!(layout.types.contains_key(base.as_str().unwrap()));
            }
            if let Some(members) = ty.other.get("members") {
                for member in members.as_array().unwrap() {
                    assert!(layout.types.contains_key(member["type"].as_str().unwrap()));
                    assert_eq!(member["slot"], "0");
                }
            }
        }
        assert!(layout.types.keys().all(|id| id.starts_with("erc7201(example.base)::")));
    }

    #[test]
    fn annotation_errors_and_non_namespaced_contracts() {
        assert!(
            layouts(
                r#"contract C {
            /// @custom:storage-location erc7201:example
            struct Data { uint256 value; }
            function invalid() public pure returns (uint256) { return true; }
        }"#,
                "C"
            )
            .is_err()
        );
        for annotation in ["erc7201:", "erc7201:two words", "erc7201"] {
            assert!(layouts(&format!("contract C {{\n/// @custom:storage-location {annotation}\nstruct Data {{ uint256 x; }}\n}}"), "C").is_err());
        }
        assert!(
            layouts(
                r#"contract C {
            /// @custom:storage-location erc7201:first
            /// @custom:storage-location erc7201:second
            struct Data { uint256 x; }
        }"#,
                "C"
            )
            .is_err()
        );
        assert!(
            layouts(
                r#"contract C {
            /// @custom:storage-location other:example
            struct Data { uint256 x; }
            uint256 value;
        }"#,
                "C"
            )
            .unwrap()
            .is_empty()
        );
        // Duplicate declarations are retained for the caller's collision policy.
        assert_eq!(
            layouts(
                r#"contract C {
            /// @custom:storage-location erc7201:same
            struct A { uint256 x; }
            /// @custom:storage-location erc7201:same
            struct B { uint256 y; }
        }"#,
                "C"
            )
            .unwrap()
            .len(),
            2
        );
    }

    #[test]
    fn root_and_annotations() {
        assert_eq!(
            erc7201_root("openzeppelin.storage.Initializable").to_string(),
            U256::from_str_radix(
                "f0c57e16840df040f15088dc2f81fe391c3923bec73e23a9662efc9c229c6a00",
                16
            )
            .unwrap()
            .to_string()
        );
        assert_eq!(parse_namespace(" erc7201:example.main \n").unwrap(), Some("example.main"));
        assert_eq!(parse_namespace("erc9999:example.main").unwrap(), None);
        for invalid in ["erc7201", "erc7201:", "erc7201:two words"] {
            assert!(parse_namespace(invalid).is_err());
        }
    }
}
