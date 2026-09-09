use super::{
    data::{ContractData, PreprocessorData},
    span_to_range,
};
use crate::fs::normalize_path;
use foundry_compilers::{
    ProjectPathsConfig, Updates,
    artifacts::{SolcLanguage, remappings::Remapping},
};
use itertools::Itertools;
use path_slash::PathExt;
use solar::sema::{
    Gcx, Hir,
    hir::{
        CallArgs, CallOptions, ContractId, Expr, ExprKind, Function, FunctionKind, StateMutability,
        Stmt, StmtKind, TypeKind, Visit,
    },
    interface::{SourceMap, data_structures::Never, source_map::FileName},
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ops::{ControlFlow, Range},
    path::{Path, PathBuf},
};

/// Holds data about referenced source contracts and bytecode dependencies.
pub(crate) struct PreprocessorDependencies {
    // Mapping contract id to preprocess -> contract bytecode dependencies.
    pub preprocessed_contracts: BTreeMap<ContractId, Vec<BytecodeDependency>>,
    // Referenced contract ids.
    pub referenced_contracts: HashSet<ContractId>,
}

impl PreprocessorDependencies {
    pub fn new(
        gcx: Gcx<'_>,
        paths: &[PathBuf],
        script_paths: &HashSet<PathBuf>,
        project_paths: &ProjectPathsConfig<SolcLanguage>,
        source_units: &[PathBuf],
        mocks: &mut HashSet<PathBuf>,
    ) -> Self {
        let relative_paths = project_paths.paths_relative();
        let src_dir = &relative_paths.sources;
        let root_dir = &project_paths.root;
        let remappings = &project_paths.remappings;
        let mut preprocessed_contracts = BTreeMap::new();
        let mut referenced_contracts = HashSet::new();
        let mut current_mocks = HashSet::new();
        let mut candidate_files = HashSet::new();

        // Helper closure for iterating candidate contracts to preprocess (tests and scripts).
        let candidate_contracts = || {
            gcx.hir.contract_ids().filter_map(|id| {
                let contract = gcx.hir.contract(id);
                let source = gcx.hir.source(contract.source);
                let FileName::Real(path) = &source.file.name else {
                    return None;
                };

                if !paths.contains(path) {
                    trace!("{} is not test or script", path.display());
                    return None;
                }

                Some((id, contract, source, path))
            })
        };

        // Collect current mocks.
        for (_, contract, _, path) in candidate_contracts() {
            if contract.linearized_bases.iter().any(|base_id| {
                let base = gcx.hir.contract(*base_id);
                matches!(
                    &gcx.hir.source(base.source).file.name,
                    FileName::Real(base_path)
                        if is_path_in_dir(base_path, src_dir, root_dir)
                )
            }) {
                let mock_path = root_dir.join(path);
                trace!("found mock contract {}", mock_path.display());
                current_mocks.insert(mock_path);
            }
        }

        // Collect dependencies for non-mock test/script contracts.
        for (contract_id, contract, source, path) in candidate_contracts() {
            let full_path = root_dir.join(path);
            candidate_files.insert(full_path.clone());

            if current_mocks.contains(&full_path) {
                trace!("{} is a mock, skipping", path.display());
                continue;
            }

            // Treat the contract as a script when its file lives under the configured script
            // directory, or when it inherits from a `Script` base (forge-std). The inheritance
            // check covers atypical layouts where script contracts are placed under `src/`.
            let is_script = script_paths.contains(path)
                || contract
                    .linearized_bases
                    .iter()
                    .skip(1)
                    .any(|base_id| gcx.hir.contract(*base_id).name.as_str() == "Script");
            let mut deps_collector = BytecodeDependencyCollector::new(
                gcx,
                source.file.src.as_str(),
                src_dir,
                root_dir,
                is_script,
            );
            // Analyze current contract.
            let _ = deps_collector.walk_contract(contract);
            let keep_native = (!deps_collector.dependencies.is_empty()
                && mocks.contains(&full_path))
                || deps_collector.dependencies.iter().any(|dependency| {
                    let dependency_id = dependency.referenced_contract;
                    let dependency = gcx.hir.contract(dependency_id);
                    let dependency_source = gcx.hir.source(dependency.source);
                    let FileName::Real(dependency_path) = &dependency_source.file.name else {
                        return true;
                    };
                    let has_constructor_args = dependency
                        .ctor
                        .is_some_and(|ctor_id| !gcx.hir.function(ctor_id).parameters.is_empty());
                    !can_rewrite(
                        dependency_path,
                        path,
                        root_dir,
                        source_units,
                        remappings,
                        has_constructor_args,
                        dependency_id,
                    )
                });
            if keep_native {
                trace!("{} has an unsafe bytecode dependency, keeping it native", path.display());
                current_mocks.insert(full_path.clone());
                preprocessed_contracts.retain(|contract_id, _| {
                    let source = gcx.hir.source(gcx.hir.contract(*contract_id).source);
                    !matches!(&source.file.name, FileName::Real(path) if root_dir.join(path) == full_path)
                });
                continue;
            }
            // Ignore empty test contracts declared in source files with other contracts.
            if !deps_collector.dependencies.is_empty() {
                preprocessed_contracts.insert(contract_id, deps_collector.dependencies);
            }
        }

        // Replace classifications only for files examined in this compiler job. This clears stale
        // mocks after a file is refactored while preserving fallback state across narrower jobs.
        for file in candidate_files {
            mocks.remove(&file);
        }
        mocks.extend(current_mocks);

        for dependencies in preprocessed_contracts.values() {
            referenced_contracts.extend(dependencies.iter().map(|dep| dep.referenced_contract));
        }

        Self { preprocessed_contracts, referenced_contracts }
    }
}

/// Represents a bytecode dependency kind.
#[derive(Debug)]
enum BytecodeDependencyKind {
    /// `type(Contract).creationCode`
    CreationCode,
    /// `new Contract`.
    New {
        /// Contract name.
        name: String,
        /// Constructor args length.
        args_length: usize,
        /// Constructor call args offset.
        call_args_offset: usize,
        /// `msg.value` (if any) used when creating contract.
        value: Option<String>,
        /// `salt` (if any) used when creating contract.
        salt: Option<String>,
        /// Whether it's a try contract creation statement, with custom return.
        try_stmt: Option<bool>,
    },
}

/// Represents a single bytecode dependency.
#[derive(Debug)]
pub(crate) struct BytecodeDependency {
    /// Dependency kind.
    kind: BytecodeDependencyKind,
    /// Source map location of this dependency.
    loc: Range<usize>,
    /// HIR id of referenced contract.
    referenced_contract: ContractId,
}

/// Walks over contract HIR and collects [`BytecodeDependency`]s and referenced contracts.
struct BytecodeDependencyCollector<'gcx, 'src> {
    /// Source map, used for determining contract item locations.
    gcx: Gcx<'gcx>,
    /// Source content of current contract.
    src: &'src str,
    /// Project source dir, used to determine if referenced contract is a source contract.
    src_dir: &'src Path,
    /// Project root, used to compare relative and absolute source paths.
    root_dir: &'src Path,
    /// Whether the contract being analyzed lives in a script file.
    /// Script bytecode references must not be rewritten: native script CREATE/CREATE2 frames
    /// are handled by the script execution inspector, and `type(Contract).creationCode` must keep
    /// its native mutability semantics.
    is_script: bool,
    /// Whether `type(Contract).creationCode` should keep native Solidity semantics.
    preserve_native_creation_code: bool,
    /// Dependencies collected for current contract.
    dependencies: Vec<BytecodeDependency>,
}

impl<'gcx, 'src> BytecodeDependencyCollector<'gcx, 'src> {
    const fn new(
        gcx: Gcx<'gcx>,
        src: &'src str,
        src_dir: &'src Path,
        root_dir: &'src Path,
        is_script: bool,
    ) -> Self {
        Self {
            gcx,
            src,
            src_dir,
            root_dir,
            is_script,
            preserve_native_creation_code: false,
            dependencies: vec![],
        }
    }

    /// Collects reference identified as bytecode dependency of analyzed contract.
    /// Discards any reference that is not in project src directory (e.g. external
    /// libraries or mock contracts that extend source contracts).
    fn collect_dependency(&mut self, dependency: BytecodeDependency) {
        // Script bytecode references must not be rewritten. See field doc on `is_script`.
        if self.is_script {
            match &dependency.kind {
                BytecodeDependencyKind::CreationCode => {
                    trace!("skip creationCode in script");
                    return;
                }
                BytecodeDependencyKind::New { .. } => {
                    trace!("skip new-expression in script");
                    return;
                }
            }
        }

        // `type(Contract).creationCode` has native `pure` semantics. Rewriting it to a `view`
        // cheatcode call would make valid pure functions fail to compile.
        if self.preserve_native_creation_code
            && matches!(&dependency.kind, BytecodeDependencyKind::CreationCode)
        {
            trace!("skip creationCode in native creationCode context");
            return;
        }

        let contract = self.gcx.hir.contract(dependency.referenced_contract);
        let has_constructor_args = contract
            .ctor
            .is_some_and(|ctor_id| !self.gcx.hir.function(ctor_id).parameters.is_empty());
        // Solidity only permits a custom layout on the most-derived contract, so the generated
        // constructor helper cannot inherit a target that declares one; keep this dependency
        // native.
        if contract.layout.is_some() && has_constructor_args {
            trace!("skip dependency on custom-layout contract");
            return;
        }

        let source = self.gcx.hir.source(contract.source);
        let FileName::Real(path) = &source.file.name else {
            return;
        };

        // Remapped imports can have absolute or symlinked paths, while compiler input paths are
        // relative and configured source directories can be canonicalized.
        if !is_path_in_dir(path, self.src_dir, self.root_dir) {
            let path = path.display();
            trace!("ignore dependency {path}");
            return;
        }

        self.dependencies.push(dependency);
    }
}

/// Returns whether generated helper and artifact references preserve the source-unit identity.
fn can_rewrite(
    path: &Path,
    source_path: &Path,
    root_dir: &Path,
    source_units: &[PathBuf],
    remappings: &[Remapping],
    has_constructor_args: bool,
    contract_id: ContractId,
) -> bool {
    let generated_path = path.strip_prefix(root_dir).unwrap_or(path);
    if !source_units.iter().any(|source_unit| source_unit == generated_path)
        || source_units.iter().filter(|source_unit| source_unit.ends_with(generated_path)).count()
            != 1
    {
        return false;
    }

    // Runtime artifact lookup uses the running test's context, which can differ from the source
    // containing an inherited helper. Any remapping matching the generated path is therefore
    // unsafe unless every possible runtime context is known.
    if remappings.iter().any(|remapping| remapping_matches_path(remapping, generated_path)) {
        return false;
    }

    if !has_constructor_args {
        return true;
    }

    let helper_path = PathBuf::from(format!("foundry-pp/DeployHelper{}.sol", contract_id.index()));
    !remappings.iter().any(|remapping| {
        // The test imports the generated helper, which in turn imports the dependency.
        remapping_applies(remapping, &helper_path, source_path, root_dir)
            || remapping_applies(remapping, generated_path, &helper_path, root_dir)
    })
}

/// Returns whether `path` resolves within `dir`, accepting relative, absolute, and symlinked paths.
fn is_path_in_dir(path: &Path, dir: &Path, root_dir: &Path) -> bool {
    let path = normalize_path(&root_dir.join(path));
    let dir = normalize_path(&root_dir.join(dir));
    path.starts_with(&dir)
        || dunce::canonicalize(path)
            .is_ok_and(|path| dunce::canonicalize(dir).is_ok_and(|dir| path.starts_with(dir)))
}

/// Returns whether a generated import would be redirected by `remapping`.
fn remapping_applies(
    remapping: &Remapping,
    import_path: &Path,
    source_unit: &Path,
    root_dir: &Path,
) -> bool {
    let source_unit = source_unit.strip_prefix(root_dir).unwrap_or(source_unit).to_slash_lossy();
    remapping
        .context
        .as_ref()
        .is_none_or(|context| source_unit.starts_with(Path::new(context).to_slash_lossy().as_ref()))
        && remapping_matches_path(remapping, import_path)
}

/// Returns whether `path` has the string prefix selected by `remapping`.
fn remapping_matches_path(remapping: &Remapping, path: &Path) -> bool {
    path.to_slash_lossy().starts_with(&remapping.name)
}

impl<'gcx> Visit<'gcx> for BytecodeDependencyCollector<'gcx, '_> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_function(&mut self, func: &'gcx Function<'gcx>) -> ControlFlow<Self::BreakValue> {
        let previous = self.preserve_native_creation_code;
        self.preserve_native_creation_code = previous
            || func.state_mutability == StateMutability::Pure
            || matches!(func.kind, FunctionKind::Modifier);
        self.walk_function(func)?;
        self.preserve_native_creation_code = previous;
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        #[allow(clippy::collapsible_match)]
        match &expr.kind {
            ExprKind::Call(call_expr, call_args, named_args) => {
                if let Some(dependency) = handle_call_expr(
                    self.src,
                    self.gcx.sess.source_map(),
                    expr,
                    call_expr,
                    call_args,
                    named_args,
                ) {
                    self.collect_dependency(dependency);
                }
            }
            ExprKind::Member(member_expr, ident) => {
                if let ExprKind::TypeCall(ty) = &member_expr.kind
                    && let TypeKind::Custom(contract_id) = &ty.kind
                    && ident.name.as_str() == "creationCode"
                    && let Some(contract_id) = contract_id.as_contract()
                {
                    self.collect_dependency(BytecodeDependency {
                        kind: BytecodeDependencyKind::CreationCode,
                        loc: span_to_range(self.gcx.sess.source_map(), expr.span),
                        referenced_contract: contract_id,
                    });
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let StmtKind::Try(stmt_try) = stmt.kind
            && let ExprKind::Call(call_expr, call_args, named_args) = &stmt_try.expr.kind
            && let Some(mut dependency) = handle_call_expr(
                self.src,
                self.gcx.sess.source_map(),
                &stmt_try.expr,
                call_expr,
                call_args,
                named_args,
            )
        {
            let has_custom_return = if let Some(clause) = stmt_try.clauses.first()
                && clause.args.len() == 1
                && let Some(ret_var) = clause.args.first()
                && let TypeKind::Custom(_) = self.hir().variable(*ret_var).ty.kind
            {
                true
            } else {
                false
            };

            if let BytecodeDependencyKind::New { try_stmt, .. } = &mut dependency.kind {
                *try_stmt = Some(has_custom_return);
            }
            self.collect_dependency(dependency);

            for clause in stmt_try.clauses {
                for &var in clause.args {
                    self.visit_nested_var(var)?;
                }
                for stmt in clause.block.stmts {
                    self.visit_stmt(stmt)?;
                }
            }
            return ControlFlow::Continue(());
        }
        self.walk_stmt(stmt)
    }
}

/// Helper function to analyze and extract bytecode dependency from a given call expression.
fn handle_call_expr(
    src: &str,
    source_map: &SourceMap,
    parent_expr: &Expr<'_>,
    call_expr: &Expr<'_>,
    call_args: &CallArgs<'_>,
    call_options: &Option<&CallOptions<'_>>,
) -> Option<BytecodeDependency> {
    if let ExprKind::New(ty_new) = &call_expr.kind
        && let TypeKind::Custom(item_id) = ty_new.kind
        && let Some(contract_id) = item_id.as_contract()
    {
        let name_loc = span_to_range(source_map, ty_new.span);
        let name = &src[name_loc];

        // Calculate offset to remove named args, e.g. for an expression like
        // `new Counter {value: 333} (  address(this))`
        // the offset will be used to replace `{value: 333} (  ` with `(`
        let call_args_offset = if call_options.is_some() && !call_args.is_empty() {
            (call_args.span.lo() - ty_new.span.hi()).to_usize()
        } else {
            0
        };

        let args_len = parent_expr.span.hi() - ty_new.span.hi();
        return Some(BytecodeDependency {
            kind: BytecodeDependencyKind::New {
                name: name.to_string(),
                args_length: args_len.to_usize(),
                call_args_offset,
                value: named_arg(src, call_options, "value", source_map),
                salt: named_arg(src, call_options, "salt", source_map),
                try_stmt: None,
            },
            loc: span_to_range(source_map, call_expr.span),
            referenced_contract: contract_id,
        });
    }
    None
}

/// Helper function to extract value of a given named arg.
fn named_arg(
    src: &str,
    call_options: &Option<&CallOptions<'_>>,
    arg: &str,
    source_map: &SourceMap,
) -> Option<String> {
    call_options
        .map(|options| options.args)
        .unwrap_or_default()
        .iter()
        .find(|named_arg| named_arg.name.as_str() == arg)
        .map(|named_arg| {
            let named_arg_loc = span_to_range(source_map, named_arg.value.span);
            src[named_arg_loc].to_string()
        })
}

/// Goes over all test/script files and replaces bytecode dependencies with cheatcode
/// invocations.
///
/// Special handling of try/catch statements with custom returns, where the try statement becomes
/// ```solidity
/// try this.addressToCounter() returns (Counter c)
/// ```
/// and helper to cast address is appended
/// ```solidity
/// function addressToCounter(address addr) returns (Counter) {
///     return Counter(addr);
/// }
/// ```
pub(crate) fn remove_bytecode_dependencies(
    gcx: Gcx<'_>,
    deps: &PreprocessorDependencies,
    data: &PreprocessorData,
) -> Updates {
    let mut updates = Updates::default();
    for (contract_id, deps) in &deps.preprocessed_contracts {
        let contract = gcx.hir.contract(*contract_id);
        let source = gcx.hir.source(contract.source);
        let FileName::Real(path) = &source.file.name else {
            continue;
        };

        let updates = updates.entry(path.clone()).or_default();
        let mut used_helpers = BTreeSet::new();

        let vm_interface_name = format!("VmContractHelper{}", contract_id.index());
        // `address(uint160(uint256(keccak256("hevm cheat code"))))`
        let vm = format!("{vm_interface_name}(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D)");
        let mut try_catch_helpers: HashSet<&str> = HashSet::default();

        for dep in deps {
            let Some(ContractData { artifact, constructor_data, .. }) =
                data.get(&dep.referenced_contract)
            else {
                continue;
            };

            match &dep.kind {
                BytecodeDependencyKind::CreationCode => {
                    // for creation code we need to just call getCode
                    updates.insert((
                        dep.loc.start,
                        dep.loc.end,
                        format!("{vm}.getCode(\"{artifact}\")"),
                    ));
                }
                BytecodeDependencyKind::New {
                    name,
                    args_length,
                    call_args_offset,
                    value,
                    salt,
                    try_stmt,
                } => {
                    let (mut update, closing_seq) = if let Some(has_ret) = try_stmt {
                        if *has_ret {
                            // try this.addressToCounter1() returns (Counter c)
                            try_catch_helpers.insert(name);
                            (format!("this.addressTo{name}{id}(", id = contract_id.index()), "}))")
                        } else {
                            (String::new(), "})")
                        }
                    } else {
                        (format!("{name}(payable("), "})))")
                    };
                    update.push_str(&format!("{vm}.deployCode({{"));
                    update.push_str(&format!("_artifact: \"{artifact}\""));

                    if let Some(value) = value {
                        update.push_str(", ");
                        update.push_str(&format!("_value: {value}"));
                    }

                    if let Some(salt) = salt {
                        update.push_str(", ");
                        update.push_str(&format!("_salt: {salt}"));
                    }

                    if constructor_data.is_some() {
                        // Insert our helper.
                        used_helpers.insert(dep.referenced_contract);

                        update.push_str(", ");
                        update.push_str(&format!(
                            "_args: encodeArgs{id}(DeployHelper{id}.FoundryPpConstructorArgs",
                            id = dep.referenced_contract.index()
                        ));
                        updates.insert((dep.loc.start, dep.loc.end + call_args_offset, update));

                        updates.insert((
                            dep.loc.end + args_length,
                            dep.loc.end + args_length,
                            format!("){closing_seq}"),
                        ));
                    } else {
                        update.push_str(closing_seq);
                        updates.insert((dep.loc.start, dep.loc.end + args_length, update));
                    }
                }
            };
        }

        // Add try catch statements after last function of the test contract.
        if !try_catch_helpers.is_empty()
            && let Some(last_fn_id) = contract.functions().last()
        {
            let last_fn_range =
                span_to_range(gcx.sess.source_map(), gcx.hir.function(last_fn_id).span);
            let to_address_fns = try_catch_helpers
                .iter()
                .map(|ty| {
                    format!(
                        r#"
                            function addressTo{ty}{id}(address addr) public pure returns ({ty}) {{
                                return {ty}(addr);
                            }}
                        "#,
                        id = contract_id.index()
                    )
                })
                .collect::<String>();

            updates.insert((last_fn_range.end, last_fn_range.end, to_address_fns));
        }

        let helper_imports = used_helpers.into_iter().map(|id| {
            let id = id.index();
            format!(
                "import {{DeployHelper{id}, encodeArgs{id}}} from \"foundry-pp/DeployHelper{id}.sol\";",
            )
        }).join("\n");
        updates.insert((
            source.file.src.len(),
            source.file.src.len(),
            format!(
                r#"
{helper_imports}

interface {vm_interface_name} {{
    function deployCode(string memory _artifact) external returns (address);
    function deployCode(string memory _artifact, bytes32 _salt) external returns (address);
    function deployCode(string memory _artifact, bytes memory _args) external returns (address);
    function deployCode(string memory _artifact, bytes memory _args, bytes32 _salt) external returns (address);
    function deployCode(string memory _artifact, uint256 _value) external returns (address);
    function deployCode(string memory _artifact, uint256 _value, bytes32 _salt) external returns (address);
    function deployCode(string memory _artifact, bytes memory _args, uint256 _value) external returns (address);
    function deployCode(string memory _artifact, bytes memory _args, uint256 _value, bytes32 _salt) external returns (address);
    function getCode(string memory _artifact) external view returns (bytes memory);
}}"#
            ),
        ));
    }
    updates
}
