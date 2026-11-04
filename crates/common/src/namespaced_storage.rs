//! ERC-7201 namespaced storage layouts.
//!
//! solc's `storageLayout` output only covers conventional storage. Namespaces declared with
//! `@custom:storage-location erc7201:<id>` are computed with Solar and appended to it, so storage
//! decoding labels namespaced fields as `<id>.<field>`.

use alloy_primitives::U256;
use eyre::Result;
use foundry_compilers::{
    ArtifactId, ProjectCompileOutput,
    artifacts::{Remapping, Sources, StorageLayout},
};
use path_slash::PathExt;
use solar::{
    ast::NatSpecKind,
    interface::{
        ColorChoice, Session,
        config::{CompileOpts, ImportRemapping},
        source_map::{FileLoader, FileName, SourceMap},
    },
    sema::{Compiler, CompilerRef, Gcx, hir, output::StorageLayoutOutput},
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Returns the storage layouts of `output`'s artifacts with ERC-7201 namespaces appended.
///
/// See [`extend_storage_layouts`].
pub fn project_storage_layouts(
    output: &mut ProjectCompileOutput,
    root: &Path,
) -> Vec<(ArtifactId, StorageLayout)> {
    let layouts = output
        .artifact_ids()
        .filter_map(|(id, artifact)| {
            artifact.storage_layout.as_ref().map(|layout| (id, layout.clone()))
        })
        .collect::<Vec<_>>();
    extend_storage_layouts(output.parser_mut().solc_mut().compiler_mut(), root, layouts)
}

/// Returns `layouts` with the ERC-7201 namespaces of each contract appended.
///
/// `compiler` must contain the parsed sources of the artifacts; it is lowered if necessary. Source
/// names are made relative to `root` to match solc's. Namespaces are optional decoding metadata, so
/// failures are logged and leave the affected layouts unchanged.
pub fn extend_storage_layouts(
    compiler: &mut Compiler,
    root: &Path,
    layouts: impl IntoIterator<Item = (ArtifactId, StorageLayout)>,
) -> Vec<(ArtifactId, StorageLayout)> {
    let mut layouts = layouts.into_iter().collect::<Vec<_>>();
    if layouts.is_empty() || !has_annotations(compiler) {
        return layouts;
    }
    let result = compiler.enter_mut(|compiler| -> Result<()> {
        lower(compiler)?;
        let gcx = compiler.gcx();
        let contracts = contracts_by_source(gcx, root);
        for (id, layout) in &mut layouts {
            let source = id.source.strip_prefix(root).unwrap_or(&id.source).to_slash_lossy();
            // Artifact names may include compiler version and profile suffixes.
            let name = id.name.split('.').next().unwrap();
            let Some(&contract) = contracts.get(&(source.into_owned(), name.to_owned())) else {
                continue;
            };
            if let Err(err) = extend_storage_layout(layout, gcx, root, contract) {
                warn!(target: "erc7201", artifact = %id.identifier(), %err, "skipping ERC-7201 namespaces");
            }
        }
        Ok(())
    });
    if let Err(err) = result {
        warn!(target: "erc7201", %err, "skipping ERC-7201 namespaces");
    }
    layouts
}

/// Appends the ERC-7201 namespaces of `contract` and its bases to its storage layout.
///
/// Namespaced fields are labeled `<namespace>.<field>`. Malformed annotations, duplicate namespace
/// roots, and errors emitted while computing types are errors, in which case `layout` is unchanged.
pub fn extend_storage_layout(
    layout: &mut StorageLayout,
    gcx: Gcx<'_>,
    root: &Path,
    contract: hir::ContractId,
) -> Result<()> {
    // Only errors emitted for this contract count; earlier ones may come from unrelated sources.
    let errors = gcx.sess.dcx.err_count();
    let namespaces = collect_storage_layouts(gcx, root, contract)?;
    if gcx.sess.dcx.err_count() > errors {
        return Err(solar_error(gcx.sess));
    }
    merge_storage_namespaces(layout, namespaces)
}

/// Lowers the parsed sources to HIR unless that has already happened.
///
/// Only the HIR and the types reachable from namespaced structs are needed, so this does not run
/// full semantic analysis; solc has already validated the sources. Diagnostics emitted while
/// lowering unrelated sources are not errors here.
pub fn lower(compiler: &mut CompilerRef<'_>) -> Result<()> {
    if compiler.gcx().stage().is_none_or(|stage| stage.is_parsing())
        && !matches!(compiler.lower_asts(), Ok(ControlFlow::Continue(())))
    {
        return Err(solar_error(compiler.sess()));
    }
    Ok(())
}

/// Parses Standard JSON `sources`, keyed by source unit name, without touching the filesystem.
///
/// Returns `None` if no source declares an ERC-7201 namespace.
pub fn parse_standard_json(sources: &Sources, remappings: &[Remapping]) -> Option<Compiler> {
    if !sources.values().any(|source| has_annotation(&source.content)) {
        return None;
    }
    let source_map = Arc::new(SourceMap::empty());
    source_map.set_file_loader(NoFileLoader);
    let opts = CompileOpts {
        standard_json: true,
        import_remappings: remappings
            .iter()
            .filter_map(|remapping| remapping.to_string().parse::<ImportRemapping>().ok())
            .collect(),
        ..Default::default()
    };
    let sess = Session::builder()
        .source_map(source_map)
        .with_buffer_emitter(ColorChoice::Never)
        .opts(opts)
        .build();
    let mut compiler = Compiler::new(sess);
    let _ = compiler.enter_mut(|compiler| -> solar::interface::Result {
        let mut pcx = compiler.parse();
        pcx.par_load_files_with_contents(
            sources
                .iter()
                .map(|(path, source)| (path.clone(), source.content.as_str().to_owned()))
                .collect::<Vec<_>>(),
        )?;
        pcx.parse();
        Ok(())
    });
    Some(compiler)
}

/// Returns the contract `name` declared in the source `path`, relative to `root`.
pub fn find_contract(
    gcx: Gcx<'_>,
    root: &Path,
    path: &Path,
    name: &str,
) -> Option<hir::ContractId> {
    let source = path.strip_prefix(root).unwrap_or(path).to_slash_lossy().into_owned();
    contracts_by_source(gcx, root).remove(&(source, name.to_owned()))
}

/// The NatSpec tag content that marks a namespace; cheap to search for in source text.
const ANNOTATION: &str = "erc7201";

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

/// Rejects filesystem access, so Standard JSON imports only resolve to the supplied sources.
struct NoFileLoader;

impl FileLoader for NoFileLoader {
    fn canonicalize_path(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }

    fn load_stdin(&self) -> io::Result<String> {
        Err(io::ErrorKind::Unsupported.into())
    }

    fn load_file(&self, _path: &Path) -> io::Result<String> {
        Err(io::ErrorKind::NotFound.into())
    }

    fn load_binary_file(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::NotFound.into())
    }
}

/// Returns whether any parsed source mentions an ERC-7201 storage location.
pub fn has_annotations(compiler: &Compiler) -> bool {
    compiler.sess().source_map().files().iter().any(|file| has_annotation(&file.src))
}

/// Returns whether `source` mentions an ERC-7201 storage location.
fn has_annotation(source: &str) -> bool {
    source.contains(ANNOTATION)
}

/// Maps `(source, name)` to contracts, with sources relative to `root` and slash-separated.
fn contracts_by_source(gcx: Gcx<'_>, root: &Path) -> HashMap<(String, String), hir::ContractId> {
    gcx.hir
        .contract_ids()
        .filter_map(|id| {
            let contract = gcx.hir.contract(id);
            let FileName::Real(path) = &gcx.hir.source(contract.source).file.name else {
                return None;
            };
            let source = path.strip_prefix(root).unwrap_or(path).to_slash_lossy().into_owned();
            Some(((source, contract.name.to_string()), id))
        })
        .collect()
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
            let root = crate::erc7201(namespace).into();
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
            annotation.split_whitespace().next() != Some(ANNOTATION),
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

/// Merges namespaces, rejecting duplicate declarations before `layout` is modified.
///
/// Type IDs are scoped to their namespace, so they can't collide with each other or with solc's.
fn merge_storage_namespaces(
    layout: &mut StorageLayout,
    namespaces: Vec<StorageNamespace>,
) -> Result<()> {
    let mut declarations = BTreeMap::new();
    for namespace in &namespaces {
        if let Some(previous) = declarations.insert(namespace.root, &namespace.declaration) {
            eyre::bail!(
                "Duplicate ERC-7201 namespace `{}` in `{previous}` and `{}`",
                namespace.id,
                namespace.declaration
            );
        }
    }
    for namespace in namespaces {
        layout.types.extend(namespace.layout.types);
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

    /// Lowers `compiler` and collects the namespaces of `name` in `path`.
    fn erc7201_storage_layouts(
        compiler: &mut Compiler,
        root: &Path,
        path: &Path,
        name: Option<&str>,
    ) -> Result<Vec<StorageNamespace>> {
        compiler.enter_mut(|compiler| {
            lower(compiler)?;
            let gcx = compiler.gcx();
            let contract = find_contract(gcx, root, path, name.unwrap()).unwrap();
            let namespaces = collect_storage_layouts(gcx, root, contract)?;
            if gcx.sess.dcx.has_errors().is_err() {
                return Err(solar_error(gcx.sess));
            }
            Ok(namespaces)
        })
    }

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
    fn annotations() {
        assert_eq!(parse_namespace(" erc7201:example.main \n").unwrap(), Some("example.main"));
        assert_eq!(parse_namespace("erc9999:example.main").unwrap(), None);
        for invalid in ["erc7201", "erc7201:", "erc7201:two words"] {
            assert!(parse_namespace(invalid).is_err());
        }
    }

    #[test]
    fn duplicate_namespaces_leave_layout_unchanged() {
        let namespace = |id: &str, root: u64| StorageNamespace {
            id: id.into(),
            declaration: format!("test.sol:C.{id}"),
            root: U256::from(root),
            layout: serde_json::from_value(serde_json::json!({
                "storage": [{
                    "astId": 1, "contract": "test.sol:C", "label": "x", "offset": 0,
                    "slot": root.to_string(), "type": format!("erc7201({id})::t_uint256")
                }],
                "types": {format!("erc7201({id})::t_uint256"): {
                    "encoding": "inplace", "label": "uint256", "numberOfBytes": "32"
                }}
            }))
            .unwrap(),
        };
        let mut layout = StorageLayout::default();
        assert_eq!(
            merge_storage_namespaces(
                &mut layout,
                vec![namespace("a", 1), namespace("b", 2), namespace("c", 1)]
            )
            .unwrap_err()
            .to_string(),
            "Duplicate ERC-7201 namespace `c` in `test.sol:C.a` and `test.sol:C.c`"
        );
        assert_eq!(layout, StorageLayout::default());

        merge_storage_namespaces(&mut layout, vec![namespace("a", 1), namespace("b", 2)]).unwrap();
        assert_eq!(
            layout.storage.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["a.x", "b.x"]
        );
        assert_eq!(layout.types.len(), 2);
    }

    #[test]
    fn artifact_layouts() {
        let mut parser = parser_at(
            r#"contract C {
            /// @custom:storage-location erc7201:example
            struct Data { uint256 value; }
        }
        contract Plain { uint256 value; }"#,
            "/project/src/test.sol",
        );
        let id = |name: &str| ArtifactId {
            path: PathBuf::new(),
            name: name.to_owned(),
            source: PathBuf::from("src/test.sol"),
            version: semver::Version::new(0, 8, 30),
            build_id: String::new(),
            profile: String::new(),
        };
        let layouts = extend_storage_layouts(
            &mut parser,
            Path::new("/project"),
            [
                (id("C"), StorageLayout::default()),
                (id("C.via-ir"), StorageLayout::default()),
                (id("C.0.8.30"), StorageLayout::default()),
                (id("C.0.8.30.via-ir"), StorageLayout::default()),
                (id("Plain"), StorageLayout::default()),
            ],
        );
        assert_eq!(
            layouts
                .iter()
                .map(|(id, layout)| (
                    id.name.as_str(),
                    layout.storage.iter().map(|s| s.label.as_str()).collect::<Vec<_>>()
                ))
                .collect::<Vec<_>>(),
            [
                ("C", vec!["example.value"]),
                ("C.via-ir", vec!["example.value"]),
                ("C.0.8.30", vec!["example.value"]),
                ("C.0.8.30.via-ir", vec!["example.value"]),
                ("Plain", vec![]),
            ]
        );
        for (_, layout) in &layouts[..4] {
            assert_eq!(layout.storage[0].contract, "src/test.sol:C");
        }
    }

    #[test]
    fn standard_json_sources() {
        let sources = Sources::from_iter([
            (
                PathBuf::from("src/Token.sol"),
                foundry_compilers::artifacts::Source::new(
                    r#"import "@oz/Base.sol";
                    contract Token is Base {}"#,
                ),
            ),
            (
                PathBuf::from("lib/oz/Base.sol"),
                foundry_compilers::artifacts::Source::new(
                    r#"contract Base {
                    /// @custom:storage-location erc7201:openzeppelin.storage.ERC20
                    struct ERC20Storage { mapping(address => uint256) balances; uint256 totalSupply; }
                }"#,
                ),
            ),
        ]);
        let remappings = ["@oz/=lib/oz/".parse::<Remapping>().unwrap()];
        let mut compiler = parse_standard_json(&sources, &remappings).unwrap();
        let mut layout = StorageLayout::default();
        compiler
            .enter_mut(|compiler| {
                lower(compiler)?;
                let gcx = compiler.gcx();
                let contract =
                    find_contract(gcx, Path::new(""), Path::new("src/Token.sol"), "Token").unwrap();
                extend_storage_layout(&mut layout, gcx, Path::new(""), contract)
            })
            .unwrap();
        assert_eq!(
            layout
                .storage
                .iter()
                .map(|s| (s.label.as_str(), s.contract.as_str()))
                .collect::<Vec<_>>(),
            [
                ("openzeppelin.storage.ERC20.balances", "lib/oz/Base.sol:Base"),
                ("openzeppelin.storage.ERC20.totalSupply", "lib/oz/Base.sol:Base"),
            ]
        );

        let plain = Sources::from_iter([(
            PathBuf::from("src/Plain.sol"),
            foundry_compilers::artifacts::Source::new("contract Plain { uint256 x; }"),
        )]);
        assert!(parse_standard_json(&plain, &[]).is_none());
    }

    #[test]
    fn profiled_artifact_layouts_keep_source_identity() {
        let sources =
            Sources::from_iter([("Left", "left"), ("Right", "right")].map(|(file, namespace)| {
                (
                    PathBuf::from(format!("src/{file}.sol")),
                    foundry_compilers::artifacts::Source::new(format!(
                        "contract C {{
                            /// @custom:storage-location erc7201:{namespace}
                            struct Data {{ uint256 value; }}
                        }}"
                    )),
                )
            }));
        let mut compiler = parse_standard_json(&sources, &[]).unwrap();
        let ids = ["Left", "Right"].map(|file| ArtifactId {
            path: PathBuf::from(format!("out/{file}.sol/C.via-ir.json")),
            name: "C.via-ir".into(),
            source: PathBuf::from(format!("src/{file}.sol")),
            version: semver::Version::new(0, 8, 30),
            build_id: file.into(),
            profile: "via-ir".into(),
        });
        let layouts = extend_storage_layouts(
            &mut compiler,
            Path::new(""),
            ids.iter().cloned().map(|id| (id, StorageLayout::default())),
        );
        assert_eq!(
            layouts.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            ids.iter().collect::<Vec<_>>()
        );
        assert_eq!(
            layouts
                .iter()
                .map(|(_, layout)| layout
                    .storage
                    .iter()
                    .map(|s| s.label.as_str())
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [vec!["left.value"], vec!["right.value"]]
        );
    }
}
