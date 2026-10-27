//! ERC-7201 namespace discovery and conversion to solc-compatible storage layouts.
//!
//! Packing and type generation are delegated to Solar.

use super::find_target_contract;
use alloy_primitives::{U256, keccak256};
use eyre::Result;
use foundry_compilers::artifacts::StorageLayout;
use path_slash::PathExt;
use solar::{
    ast::NatSpecKind,
    sema::{
        Compiler, Gcx, hir,
        interface::{Session, source_map::FileName},
        output::StorageLayoutOutput,
    },
};
use std::{collections::BTreeMap, ops::ControlFlow, path::Path};

/// Appends the ERC-7201 namespaces of a contract and its bases to its storage layout.
///
/// Namespaced fields are labeled `<namespace>.<field>`. Duplicate namespace roots and conflicting
/// type definitions are errors.
pub fn extend_storage_layout(
    layout: &mut StorageLayout,
    compiler: &mut Compiler,
    root: &Path,
    target_path: &Path,
    target_name: Option<&str>,
) -> Result<()> {
    merge_storage_namespaces(
        layout,
        erc7201_storage_layouts(compiler, root, target_path, target_name)?,
    )
}

/// A storage namespace declared by a struct in a contract or one of its bases.
#[derive(Clone, Debug)]
struct StorageNamespace {
    /// The identifier following `erc7201:` in the storage-location annotation.
    id: String,
    /// The fully qualified name of the declaring struct.
    declaration: String,
    /// The ERC-7201 root slot.
    root: U256,
    /// The struct's fields at absolute slots, with nested members at relative slots.
    ///
    /// Type IDs are scoped to this namespace so they cannot alias solc-generated IDs.
    /// AST IDs originate from Solar and are not solc AST IDs.
    layout: StorageLayout,
}

/// Computes ERC-7201 layouts for a contract in the resolved project.
///
/// Only the HIR and the types reachable from namespaced structs are needed, so this does not run
/// full semantic analysis; solc has already validated the sources. Errors emitted while lowering
/// or computing types are returned rather than silently emitting incomplete layouts.
fn erc7201_storage_layouts(
    compiler: &mut Compiler,
    root: &Path,
    target_path: &Path,
    target_name: Option<&str>,
) -> Result<Vec<StorageNamespace>> {
    compiler.enter_mut(|compiler| {
        if compiler.gcx().stage().is_none_or(|stage| stage.is_parsing())
            && !matches!(compiler.lower_asts(), Ok(ControlFlow::Continue(())))
        {
            return Err(solar_error(compiler.sess()));
        }
        let gcx = compiler.gcx();
        if gcx.sess.dcx.has_errors().is_err() {
            return Err(solar_error(gcx.sess));
        }
        let target = find_target_contract(&gcx.hir, target_path, target_name)?;
        let namespaces = collect_storage_layouts(gcx, root, target)?;
        if gcx.sess.dcx.has_errors().is_err() {
            return Err(solar_error(gcx.sess));
        }
        Ok(namespaces)
    })
}

/// Returns namespaces declared in `contract` and its linearized bases, once per declaration.
///
/// Unrelated contracts, imported libraries, and file-level structs are excluded: importing a
/// declaration does not establish that the selected contract uses its storage. Unknown
/// storage-location schemes are ignored; malformed ERC-7201 annotations are errors.
fn collect_storage_layouts(
    gcx: Gcx<'_>,
    root: &Path,
    contract: hir::ContractId,
) -> Result<Vec<StorageNamespace>> {
    let mut namespaces = Vec::new();
    for &base in gcx.hir.contract(contract).linearized_bases.iter().rev() {
        let contract = gcx.hir.contract(base);
        for id in contract.items.iter().filter_map(|item| item.as_struct()) {
            let strukt = gcx.hir.strukt(id);
            let Some(namespace) = storage_namespace(gcx, strukt)? else { continue };
            // Match solc's project-relative source names.
            let contract_name = match &gcx.hir.source(contract.source).file.name {
                FileName::Real(path) => format!(
                    "{}:{}",
                    path.strip_prefix(root).unwrap_or(path).to_slash_lossy(),
                    contract.name
                ),
                _ => gcx.contract_fully_qualified_name(base).to_string(),
            };
            let declaration = format!("{contract_name}.{}", strukt.name);
            let root = erc7201_root(namespace);
            let mut output = gcx.storage_layout_for_struct(id, root);
            scope_layout(&mut output, namespace, &contract_name);
            namespaces.push(StorageNamespace {
                id: namespace.to_owned(),
                declaration,
                root,
                layout: serde_json::from_value(serde_json::to_value(output)?)?,
            });
        }
    }
    Ok(namespaces)
}

/// Returns the ERC-7201 namespace declared by a struct's `@custom:storage-location` tag.
fn storage_namespace<'a>(gcx: Gcx<'a>, strukt: &hir::Struct<'_>) -> Result<Option<&'a str>> {
    let mut namespace = None;
    for doc in gcx.hir.doc(strukt.doc).ast_comments().iter() {
        for tag in doc.natspec.iter() {
            if let NatSpecKind::Custom { name } = tag.kind
                && name.as_str() == "storage-location"
                && let Some(id) = parse_namespace(tag.content())?
                && namespace.replace(id).is_some()
            {
                eyre::bail!("multiple ERC-7201 annotations on struct `{}`", strukt.name);
            }
        }
    }
    Ok(namespace)
}

fn parse_namespace(annotation: &str) -> Result<Option<&str>> {
    let annotation = annotation.trim();
    let Some(id) = annotation.strip_prefix("erc7201:") else {
        eyre::ensure!(
            annotation.split_whitespace().next() != Some("erc7201"),
            "expected `erc7201:<namespace>` storage location"
        );
        return Ok(None);
    };
    eyre::ensure!(
        !id.is_empty() && !id.contains(char::is_whitespace),
        "ERC-7201 namespace must be nonempty and contain no whitespace"
    );
    Ok(Some(id))
}

/// Computes `keccak256(abi.encode(uint256(keccak256(id)) - 1)) & ~uint256(0xff)`.
fn erc7201_root(id: &str) -> U256 {
    let inner = U256::from_be_bytes(keccak256(id.as_bytes()).0).wrapping_sub(U256::from(1));
    U256::from_be_bytes(keccak256(inner.to_be_bytes::<32>()).0) & !U256::from(0xff)
}

/// Scopes type IDs to `namespace` and attributes every entry to `contract`.
fn scope_layout(output: &mut StorageLayoutOutput, namespace: &str, contract: &str) {
    let scope = |id: &mut String| *id = format!("erc7201({namespace})::{id}");
    for entry in &mut output.storage {
        scope(&mut entry.r#type);
        entry.contract = contract.to_owned();
    }
    let Some(types) = &mut output.types else { return };
    *types = std::mem::take(types)
        .into_iter()
        .map(|(mut id, mut ty)| {
            for reference in [&mut ty.base, &mut ty.key, &mut ty.value].into_iter().flatten() {
                scope(reference);
            }
            for member in &mut ty.members {
                scope(&mut member.r#type);
                member.contract = contract.to_owned();
            }
            scope(&mut id);
            (id, ty)
        })
        .collect();
}

/// Merges namespaces without allowing duplicate declarations or conflicting type definitions.
fn merge_storage_namespaces(
    layout: &mut StorageLayout,
    namespaces: Vec<StorageNamespace>,
) -> Result<()> {
    let mut declarations = BTreeMap::new();
    for namespace in namespaces {
        if let Some(previous) = declarations.insert(namespace.root, namespace.declaration.clone()) {
            eyre::bail!(
                "Duplicate ERC-7201 namespace `{}` in `{previous}` and `{}`",
                namespace.id,
                namespace.declaration
            );
        }
        for (id, ty) in namespace.layout.types {
            if let Some(existing) = layout.types.get(&id) {
                eyre::ensure!(
                    existing == &ty,
                    "Conflicting storage type `{id}` in namespace `{}`",
                    namespace.id
                );
            } else {
                layout.types.insert(id, ty);
            }
        }
        for mut entry in namespace.layout.storage {
            entry.label = format!("{}.{}", namespace.id, entry.label);
            layout.storage.push(entry);
        }
    }
    Ok(())
}

fn solar_error(sess: &Session) -> eyre::Error {
    let diagnostics = sess.dcx.emitted_diagnostics().map(|d| d.to_string()).unwrap_or_default();
    eyre::eyre!("Solar failed while inspecting ERC-7201 storage\n{diagnostics}")
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

    fn layouts(source: &str, contract: &str) -> Result<Vec<StorageNamespace>> {
        erc7201_storage_layouts(
            &mut parser_at(source, "test.sol"),
            Path::new(""),
            Path::new("test.sol"),
            Some(contract),
        )
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
        let namespaces = erc7201_storage_layouts(
            &mut parser,
            Path::new("/project"),
            Path::new("/project/src/test.sol"),
            Some("C"),
        )
        .unwrap();
        let namespace = &namespaces[0];
        assert_eq!(namespace.declaration, "src/test.sol:C.Data");
        assert_eq!(namespace.layout.storage[0].contract, "src/test.sol:C");
        let ty = &namespace.layout.types[&namespace.layout.storage[0].storage_type];
        assert_eq!(ty.other["members"][0]["contract"], "src/test.sol:C");
    }

    #[test]
    fn repeated_inspection() {
        let mut parser = parser_at(
            r#"contract C {
            /// @custom:storage-location erc7201:example
            struct Data { uint256 value; }
        }"#,
            "test.sol",
        );
        let mut inspect = || {
            erc7201_storage_layouts(&mut parser, Path::new(""), Path::new("test.sol"), Some("C"))
                .unwrap()
        };
        assert_eq!(inspect()[0].layout, inspect()[0].layout);
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
        // Duplicate declarations are retained for the merge's collision policy.
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

    #[test]
    fn namespace_type_collisions() {
        let layout: StorageLayout = serde_json::from_value(serde_json::json!({
            "storage": [],
            "types": {"erc7201(example)::t_uint256": {
                "encoding": "inplace", "label": "uint256", "numberOfBytes": "32"
            }}
        }))
        .unwrap();
        let namespace = StorageNamespace {
            id: "example".into(),
            declaration: "test.sol:C.Data".into(),
            root: U256::ZERO,
            layout: layout.clone(),
        };
        let mut merged = layout.clone();
        merge_storage_namespaces(&mut merged, vec![namespace.clone()]).unwrap();
        assert_eq!(merged, layout);

        let mut conflicting = namespace.clone();
        conflicting.layout.types.values_mut().next().unwrap().number_of_bytes = "16".into();
        assert_eq!(
            merge_storage_namespaces(&mut merged, vec![conflicting]).unwrap_err().to_string(),
            "Conflicting storage type `erc7201(example)::t_uint256` in namespace `example`"
        );
        assert_eq!(merged, layout);
        assert_eq!(
            merge_storage_namespaces(
                &mut StorageLayout::default(),
                vec![namespace.clone(), namespace]
            )
            .unwrap_err()
            .to_string(),
            "Duplicate ERC-7201 namespace `example` in `test.sol:C.Data` and `test.sol:C.Data`"
        );
    }
}
