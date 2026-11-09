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
    ast::{ContractKind, NatSpecKind},
    data_structures::{Never, map::FxIndexSet},
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

/// Appends the ERC-7201 namespaces of `contract`, its bases, and the libraries they reference to
/// its storage layout.
///
/// Namespaced fields are labeled `<namespace>.<field>`. Malformed annotations, duplicate namespace
/// roots, and errors emitted while computing types in the contract and its bases are errors, in
/// which case `layout` is unchanged. Such problems in libraries only skip the affected namespace.
pub fn extend_storage_layout(
    layout: &mut StorageLayout,
    gcx: Gcx<'_>,
    root: &Path,
    contract: hir::ContractId,
) -> Result<()> {
    merge_storage_namespaces(layout, collect_storage_layouts(gcx, root, contract)?)
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

/// Returns namespaces declared in `contract`, its linearized bases, and the libraries their code
/// references, once per declaration.
///
/// Unrelated contracts, unreferenced libraries, and file-level structs are excluded: importing a
/// declaration does not establish that the selected contract uses its storage. Unknown
/// storage-location schemes are ignored. Malformed ERC-7201 annotations in the contract and its
/// bases are errors. Library namespaces are best-effort: malformed ones, and ones whose root is
/// already declared by a base or an earlier library, are logged and skipped.
fn collect_storage_layouts(
    gcx: Gcx<'_>,
    root: &Path,
    contract: hir::ContractId,
) -> Result<Vec<StorageNamespace>> {
    let mut namespaces = Vec::new();
    let bases = gcx.hir.contract(contract).linearized_bases;
    for &base in bases.iter().rev() {
        for id in gcx.hir.contract(base).items.iter().filter_map(|item| item.as_struct()) {
            namespaces.extend(struct_namespace(gcx, root, base, id)?);
        }
    }
    if !has_library_annotations(gcx) {
        return Ok(namespaces);
    }
    for library in referenced_libraries(gcx, bases) {
        for id in gcx.hir.contract(library).items.iter().filter_map(|item| item.as_struct()) {
            match struct_namespace(gcx, root, library, id) {
                Ok(Some(namespace)) => {
                    if let Some(previous) = namespaces.iter().find(|n| n.root == namespace.root) {
                        warn!(
                            target: "erc7201",
                            namespace = %namespace.id,
                            declaration = %namespace.declaration,
                            previous = %previous.declaration,
                            "skipping duplicate library ERC-7201 namespace"
                        );
                    } else {
                        namespaces.push(namespace);
                    }
                }
                Ok(None) => {}
                Err(err) => warn!(
                    target: "erc7201",
                    library = %gcx.contract_fully_qualified_name(library),
                    %err,
                    "skipping library ERC-7201 namespace"
                ),
            }
        }
    }
    Ok(namespaces)
}

/// Returns the namespace declared by struct `id` in `contract`, if any.
///
/// Errors if the annotation is malformed or Solar emits errors while computing the layout.
fn struct_namespace(
    gcx: Gcx<'_>,
    root: &Path,
    contract: hir::ContractId,
    id: hir::StructId,
) -> Result<Option<StorageNamespace>> {
    let strukt = gcx.hir.strukt(id);
    let Some(namespace) = storage_namespace(gcx, strukt)? else { return Ok(None) };
    // Match solc's project-relative source names.
    let contract_name = match &gcx.hir.source(gcx.hir.contract(contract).source).file.name {
        FileName::Real(path) => format!(
            "{}:{}",
            path.strip_prefix(root).unwrap_or(path).to_slash_lossy(),
            gcx.hir.contract(contract).name
        ),
        _ => gcx.contract_fully_qualified_name(contract).to_string(),
    };
    let slot = crate::erc7201(namespace).into();
    // Only errors emitted for this struct count; earlier ones may come from unrelated sources.
    let errors = gcx.sess.dcx.err_count();
    let mut output = gcx.storage_layout_for_struct(id, slot);
    if gcx.sess.dcx.err_count() > errors {
        return Err(solar_error(gcx.sess));
    }
    scope_layout(&mut output, namespace, &contract_name);
    Ok(Some(StorageNamespace {
        id: namespace.to_owned(),
        declaration: format!("{contract_name}.{}", strukt.name),
        root: slot,
        layout: serde_json::from_value(serde_json::to_value(output)?)?,
    }))
}

/// Returns whether any library declares a struct that mentions an ERC-7201 storage location.
///
/// Finding referenced libraries walks function bodies, so it is skipped when no library could
/// contribute a namespace, such as when only base contracts declare them.
fn has_library_annotations(gcx: Gcx<'_>) -> bool {
    gcx.hir.strukt_ids().any(|id| {
        let strukt = gcx.hir.strukt(id);
        strukt
            .contract
            .is_some_and(|contract| gcx.hir.contract(contract).kind == ContractKind::Library)
            && gcx
                .hir
                .doc(strukt.doc)
                .ast_comments()
                .iter()
                .any(|doc| doc.natspec.iter().any(|tag| has_annotation(tag.content())))
    })
}

/// Returns the libraries referenced by `contracts`, directly or through other libraries and free
/// functions, in discovery order and excluding `contracts` themselves.
///
/// Library code runs against the caller's storage, so a namespace declared in a library, such as
/// `Storage.get()` returning a namespaced struct, belongs to every contract that reaches it.
/// `using ... global` directives declared in other sources are not followed.
fn referenced_libraries(gcx: Gcx<'_>, contracts: &[hir::ContractId]) -> Vec<hir::ContractId> {
    let mut references = LibraryReferences {
        gcx,
        libraries: FxIndexSet::default(),
        functions: FxIndexSet::default(),
    };
    // Most-base first, matching the order of base namespaces.
    for &contract in contracts.iter().rev() {
        references.visit_contract_code(contract);
    }
    // Code reached from libraries and free functions is appended, so this reaches a fixed point.
    let (mut libraries, mut functions) = (0, 0);
    loop {
        if let Some(&library) = references.libraries.get_index(libraries) {
            libraries += 1;
            references.visit_contract_code(library);
        } else if let Some(&function) = references.functions.get_index(functions) {
            functions += 1;
            let _ = hir::Visit::visit_nested_function(&mut references, function);
        } else {
            break;
        }
    }
    // An inspected library is its own base, and its namespaces are collected as such.
    references.libraries.into_iter().filter(|id| !contracts.contains(id)).collect()
}

/// Collects the libraries that visited code refers to by name, type, modifier, or `using`, and
/// the free functions it calls, so their bodies can be visited too.
struct LibraryReferences<'hir> {
    gcx: Gcx<'hir>,
    libraries: FxIndexSet<hir::ContractId>,
    functions: FxIndexSet<hir::FunctionId>,
}

impl<'hir> LibraryReferences<'hir> {
    /// Visits a contract's items, its storage layout specifier, and the `using` directives that
    /// apply to it.
    fn visit_contract_code(&mut self, id: hir::ContractId) {
        let contract = self.gcx.hir.contract(id);
        let _ = hir::Visit::visit_contract(self, contract);
        if let Some(layout) = contract.layout {
            let _ = hir::Visit::visit_expr(self, layout);
        }
        self.visit_usings(contract.usings);
        self.visit_usings(self.gcx.hir.source(contract.source).usings);
    }

    /// Records the library that declares `item`, `item` itself if it is a library, or `item` if
    /// it is a free function.
    fn add_owner(&mut self, item: hir::ItemId) {
        match item {
            hir::ItemId::Contract(id) => self.add(id),
            hir::ItemId::Function(id) if self.gcx.hir.function(id).contract.is_none() => {
                self.functions.insert(id);
            }
            _ => {
                if let Some(id) = self.gcx.hir.item(item).contract() {
                    self.add(id);
                }
            }
        }
    }

    fn add(&mut self, id: hir::ContractId) {
        if self.gcx.hir.contract(id).kind == ContractKind::Library {
            self.libraries.insert(id);
        }
    }

    fn visit_usings(&mut self, usings: &'hir [hir::UsingDirective<'hir>]) {
        for directive in usings {
            for entry in directive.entries {
                match entry.kind {
                    hir::UsingEntryKind::Library(library) => self.add(library),
                    hir::UsingEntryKind::Functions(functions) => {
                        for &function in functions {
                            self.add_owner(function.into());
                        }
                    }
                    hir::UsingEntryKind::Err(_) => {}
                }
            }
        }
    }
}

impl<'hir> hir::Visit<'hir> for LibraryReferences<'hir> {
    type BreakValue = Never;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let hir::ExprKind::Ident(resolutions) = expr.kind {
            for resolution in resolutions {
                if let hir::Res::Item(item) = *resolution {
                    self.add_owner(item);
                }
            }
        } else if let hir::ExprKind::Member(base, member) = expr.kind {
            let mut names = vec![member];
            let mut root = base.peel_parens();
            while let hir::ExprKind::Member(base, member) = root.kind {
                names.push(member);
                root = base.peel_parens();
            }
            if let hir::ExprKind::Ident(resolutions) = root.kind {
                names.reverse();
                for resolution in resolutions {
                    if let hir::Res::Namespace(source) = *resolution
                        && let Some(resolutions) =
                            self.gcx.source_path_resolutions(&names, source, None)
                        && let Some(targets) = resolutions.last()
                    {
                        // Keep every overload of namespace-qualified free functions.
                        for target in targets {
                            if let hir::Res::Item(item) = *target {
                                self.add_owner(item);
                            }
                        }
                    }
                }
            }
        }
        self.walk_expr(expr)
    }

    fn visit_modifier(
        &mut self,
        modifier: &'hir hir::Modifier<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        self.add_owner(modifier.id);
        self.walk_modifier(modifier)
    }

    fn visit_ty(&mut self, ty: &'hir hir::Type<'hir>) -> ControlFlow<Self::BreakValue> {
        if let hir::TypeKind::Custom(item) = ty.kind {
            self.add_owner(item);
        }
        self.walk_ty(ty)
    }

    fn visit_function(
        &mut self,
        function: &'hir hir::Function<'hir>,
    ) -> ControlFlow<Self::BreakValue> {
        if function.contract.is_none() {
            self.visit_usings(self.gcx.hir.source(function.source).usings);
        }
        self.walk_function(function)
    }
}

/// Returns the ERC-7201 namespace declared by a struct's `@custom:storage-location` tag.
pub fn storage_namespace<'a>(gcx: Gcx<'a>, strukt: &hir::Struct<'_>) -> Result<Option<&'a str>> {
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
    use foundry_compilers::artifacts::Source;

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
    fn referenced_library_namespaces() {
        let namespaces = layouts(
            r#"
            library Storage {
                /// @custom:storage-location erc7201:example.storage
                struct Data { uint256 value; }
                function get() internal pure returns (Data storage $) {
                    assembly { $.slot := 0 }
                }
            }
            library Logic {
                function bump() external { Storage.get().value += 1; }
            }
            library Math {
                /// @custom:storage-location erc7201:example.math
                struct Data { uint256 value; }
                function double(uint256 x) internal pure returns (uint256) { return x * 2; }
            }
            library Unused {
                /// @custom:storage-location erc7201:example.unused
                struct Data { uint256 value; }
            }
            abstract contract Base {
                function bump() public { Logic.bump(); }
            }
            contract C is Base {
                using Math for uint256;
            }
        "#,
            "C",
        )
        .unwrap();
        assert_eq!(
            namespaces.iter().map(|n| n.declaration.as_str()).collect::<Vec<_>>(),
            ["test.sol:Math.Data", "test.sol:Storage.Data"]
        );
    }

    fn declarations(source: &str, contract: &str) -> Vec<String> {
        layouts(source, contract).unwrap().into_iter().map(|n| n.declaration).collect()
    }

    #[test]
    fn library_artifact_namespaces() {
        let source = r#"
            library Storage {
                /// @custom:storage-location erc7201:example.storage
                struct Data { uint256 value; }
                function get() internal pure returns (Data storage $) {
                    assembly { $.slot := 0 }
                }
            }
        "#;
        assert_eq!(declarations(source, "Storage"), ["test.sol:Storage.Data"]);
    }

    #[test]
    fn library_reference_kinds() {
        const LIBRARY: &str = r#"
            library L {
                /// @custom:storage-location erc7201:lib
                struct Data { uint256 value; }
                uint256 internal constant K = 1;
                function f() internal pure returns (uint256) { return 1; }
                function g(uint256 x) internal pure returns (uint256) { return x; }
            }
        "#;
        for code in [
            "contract C { function h(L.Data storage) internal {} }",
            "contract C { modifier m(uint256) { _; } function h() public m(L.f()) {} }",
            "contract B { constructor(uint256) {} } contract C is B(L.f()) {}",
            "using L for uint256; contract C {}",
            "contract C { using {L.g} for uint256; }",
            "function free() pure returns (uint256) { return L.f(); }
             contract C { function h() public pure { free(); } }",
            "function inc(uint256 x) pure returns (uint256) { return L.g(x); }
             contract C { using {inc} for uint256; }",
            "contract C layout at L.K {}",
        ] {
            let source = format!("{LIBRARY}\n{code}");
            assert_eq!(declarations(&source, "C"), ["test.sol:L.Data"], "{code}");
        }
        assert!(declarations(&format!("{LIBRARY}\ncontract C {{}}"), "C").is_empty());
    }

    #[test]
    fn library_namespaces_are_best_effort() {
        // A base's namespace takes precedence over a library's.
        let source = r#"
            library L {
                /// @custom:storage-location erc7201:app.main
                struct Data { uint256 value; }
                function f() internal {}
            }
            contract C {
                /// @custom:storage-location erc7201:app.main
                struct Main { uint256 value; }
                function g() public { L.f(); }
            }
        "#;
        assert_eq!(declarations(source, "C"), ["test.sol:C.Main"]);

        // Of vendored copies, the first discovered library wins.
        let source = r#"
            library LibA {
                /// @custom:storage-location erc7201:app.shared
                struct Data { uint256 value; }
                function f() internal {}
            }
            library LibB {
                /// @custom:storage-location erc7201:app.shared
                struct Data { uint256 value; }
                function f() internal {}
            }
            contract C {
                /// @custom:storage-location erc7201:app.own
                struct Own { uint256 value; }
                function g() public { LibA.f(); LibB.f(); }
            }
        "#;
        assert_eq!(declarations(source, "C"), ["test.sol:C.Own", "test.sol:LibA.Data"]);

        // Malformed library annotations only skip their struct.
        let source = r#"
            library Util {
                /// @custom:storage-location erc7201
                struct Broken { uint256 value; }
                /// @custom:storage-location erc7201:util.valid
                struct Valid { uint256 value; }
                function id(uint256 x) internal pure returns (uint256) { return x; }
            }
            contract C {
                /// @custom:storage-location erc7201:app.own
                struct Own { uint256 value; }
                function g() public pure returns (uint256) { return Util.id(1); }
            }
        "#;
        assert_eq!(declarations(source, "C"), ["test.sol:C.Own", "test.sol:Util.Valid"]);
    }

    #[test]
    fn namespace_import_alias() {
        for (import, exports, accessor) in [
            ("Lib.sol", "", "L.Storage"),
            ("Exports.sol", r#"import {Storage} from "./Lib.sol";"#, "L.Storage"),
            ("Exports.sol", r#"import {Storage as Store} from "./Lib.sol";"#, "L.Store"),
            ("Exports.sol", r#"import "./Lib.sol" as Inner;"#, "L.Inner.Storage"),
            ("Exports.sol", r#"import "./Lib.sol" as Inner;"#, "(L).Inner.Storage"),
        ] {
            let sources = Sources::from_iter([
                (
                    PathBuf::from("src/Lib.sol"),
                    Source::new(
                        r#"library Storage {
                            /// @custom:storage-location erc7201:example.storage
                            struct Data { uint256 value; }
                            function get() internal pure returns (Data storage $) {
                                bytes32 slot = keccak256(abi.encode(uint256(keccak256("example.storage")) - 1)) & ~bytes32(uint256(0xff));
                                assembly { $.slot := slot }
                            }
                        }"#,
                    ),
                ),
                (PathBuf::from("src/Exports.sol"), Source::new(exports)),
                (
                    PathBuf::from("src/C.sol"),
                    Source::new(format!(
                        r#"import "./{import}" as L;
                        contract C {{ function f() public {{ {accessor}.get().value = 1; }} }}"#
                    )),
                ),
            ]);
            let mut compiler = parse_standard_json(&sources, &[]).unwrap();
            let mut layout = StorageLayout::default();
            compiler
                .enter_mut(|compiler| {
                    lower(compiler)?;
                    let gcx = compiler.gcx();
                    let contract =
                        find_contract(gcx, Path::new(""), Path::new("src/C.sol"), "C").unwrap();
                    extend_storage_layout(&mut layout, gcx, Path::new(""), contract)
                })
                .unwrap();
            assert_eq!(
                layout
                    .storage
                    .iter()
                    .map(|s| (s.label.as_str(), s.contract.as_str()))
                    .collect::<Vec<_>>(),
                [("example.storage.value", "src/Lib.sol:Storage")],
                "{import}: {exports} {accessor}"
            );
        }
    }

    #[test]
    fn free_function_source_usings() {
        for using in ["using Storage for uint256;", "using {Storage.bump} for uint256;"] {
            for (import, call) in [
                (r#"import {bump} from "./Free.sol";"#, "bump"),
                (r#"import "./Free.sol" as F;"#, "F.bump"),
                (r#"import "./Free.sol" as F;"#, "(F).bump"),
            ] {
                let sources = Sources::from_iter([
                    (
                        PathBuf::from("src/Lib.sol"),
                        Source::new(
                            r#"library Storage {
                            /// @custom:storage-location erc7201:example.storage
                            struct Data { uint256 value; }
                            function bump(uint256 x) internal returns (uint256) {
                                Data storage $;
                                bytes32 slot = keccak256(abi.encode(uint256(keccak256("example.storage")) - 1)) & ~bytes32(uint256(0xff));
                                assembly { $.slot := slot }
                                return $.value += x;
                            }
                        }"#,
                        ),
                    ),
                    (
                        PathBuf::from("src/Free.sol"),
                        Source::new(format!(
                            r#"import {{Storage}} from "./Lib.sol";
                        {using}
                        function bump(uint256 x) returns (uint256) {{ return x.bump(); }}
                        function bump(address) pure returns (uint256) {{ return 0; }}"#
                        )),
                    ),
                    (
                        PathBuf::from("src/C.sol"),
                        Source::new(format!(
                            r#"{import}
                        contract C {{
                            function f() public returns (uint256) {{ return {call}(uint256(1)); }}
                        }}
                        contract Unused {{}}"#,
                        )),
                    ),
                ]);
                let mut compiler = parse_standard_json(&sources, &[]).unwrap();
                for (name, expected) in [
                    ("C", vec![("example.storage.value", "src/Lib.sol:Storage")]),
                    ("Unused", vec![]),
                ] {
                    let mut layout = StorageLayout::default();
                    compiler
                        .enter_mut(|compiler| {
                            lower(compiler)?;
                            let gcx = compiler.gcx();
                            let contract =
                                find_contract(gcx, Path::new(""), Path::new("src/C.sol"), name)
                                    .unwrap();
                            extend_storage_layout(&mut layout, gcx, Path::new(""), contract)
                        })
                        .unwrap();
                    assert_eq!(
                        layout
                            .storage
                            .iter()
                            .map(|s| (s.label.as_str(), s.contract.as_str()))
                            .collect::<Vec<_>>(),
                        expected,
                        "{name}: {using} {import} {call}"
                    );
                }
            }
        }
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
