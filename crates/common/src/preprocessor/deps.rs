use super::{
    data::{ContractData, PreprocessorData, deploy_helper_path},
    span_to_range,
};
use crate::fs::normalize_path;
use foundry_compilers::{
    ProjectPathsConfig, Updates,
    artifacts::{SolcLanguage, remappings::Remapping},
    project::{NativeDependencies, NativeDependencyState, PreprocessorState},
};
use itertools::Itertools;
use path_slash::PathExt;
use solar::{
    ast::{ItemKind, UserDefinableOperator},
    sema::{
        Gcx, Hir,
        builtins::Builtin,
        hir::{
            CallArgs, CallArgsKind, CallOptions, Contract, ContractId, ContractKind, Expr,
            ExprKind, Function, FunctionId, FunctionKind, Modifier, Res, SourceId, StateMutability,
            Stmt, StmtKind, TypeKind, UsingDirective, UsingEntryKind, Variable, VariableId,
            Visibility, Visit,
        },
        interface::{SourceMap, Symbol, data_structures::Never, source_map::FileName},
    },
};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ops::{ControlFlow, Range},
    path::{Path, PathBuf},
};

/// Compiler and source context whose validation would be lost by rewriting construction.
#[derive(Clone, Copy)]
pub(super) struct ConstructorContext {
    pub abi_coder_v2: bool,
    pub supports_create2: bool,
}

impl ConstructorContext {
    fn for_source(mut self, gcx: Gcx<'_>, source: SourceId) -> Self {
        let ast = gcx
            .sources
            .get_file(&gcx.hir.source(source).file)
            .and_then(|(_, source)| source.ast.as_ref());
        let Some(ast) = ast else {
            self.abi_coder_v2 = false;
            return self;
        };
        for item in ast.items.iter() {
            if let ItemKind::Pragma(pragma) = &item.kind
                && let Some((name, Some(value))) = pragma.tokens.as_name_and_value()
            {
                match (name.as_str(), value.as_str()) {
                    ("abicoder", "v1") => {
                        self.abi_coder_v2 = false;
                        return self;
                    }
                    ("abicoder", "v2") | ("experimental", "ABIEncoderV2") => {
                        self.abi_coder_v2 = true;
                    }
                    _ => {}
                }
            }
        }
        self
    }
}

/// Holds data about referenced source contracts and bytecode dependencies.
pub(crate) struct PreprocessorDependencies {
    // Mapping contract id to preprocess -> contract bytecode dependencies.
    pub preprocessed_contracts: BTreeMap<ContractId, Vec<BytecodeDependency>>,
    // Referenced contract ids.
    pub referenced_contracts: HashSet<ContractId>,
}

impl PreprocessorDependencies {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        gcx: Gcx<'_>,
        constructor_context: ConstructorContext,
        paths: &[PathBuf],
        script_paths: &HashSet<PathBuf>,
        project_paths: &ProjectPathsConfig<SolcLanguage>,
        source_units: &[PathBuf],
        mocks: &mut HashSet<PathBuf>,
        preprocessor_state: &mut PreprocessorState,
    ) -> Self {
        let relative_paths = project_paths.paths_relative();
        let src_dir = &relative_paths.sources;
        let root_dir = &project_paths.root;
        let remappings = &project_paths.remappings;
        let mut preprocessed_contracts = BTreeMap::new();
        let mut referenced_contracts = HashSet::new();
        let mut current_mocks = HashSet::new();
        let mut current_native_dependencies = NativeDependencies::new();
        let candidate_files =
            paths.iter().map(|path| normalize_path(&root_dir.join(path))).collect::<HashSet<_>>();
        let mut conservative_files = HashSet::new();
        let global_using_dependencies = using_dependency_sources(
            gcx,
            gcx.hir
                .source_ids()
                .flat_map(|id| gcx.hir.source(id).usings)
                .filter(|directive| directive.global),
        );

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

        // An internal call can observe return data left by another function. Analyze the whole
        // owning contract and its reachable helpers before deciding which scopes can be rewritten.
        let mut native_return_data_contracts = HashSet::new();
        for (id, _, _, _) in candidate_contracts() {
            let mut observer = ReturnDataObserver::new(gcx);
            let _ = observer.visit_nested_contract(id);
            if observer.observes_return_data {
                native_return_data_contracts.insert(id);
                native_return_data_contracts.extend(observer.contracts);
            }
        }

        // Collect current mocks.
        for (_, contract, _, path) in candidate_contracts() {
            let full_path = normalize_path(&root_dir.join(path));
            let mut inherited_dependencies = BTreeSet::new();
            let mut inherits_source_contract = false;
            for base_id in contract.linearized_bases {
                let base = gcx.hir.contract(*base_id);
                if let FileName::Real(base_path) = &gcx.hir.source(base.source).file.name {
                    let base_path = normalize_path(&root_dir.join(base_path));
                    let is_source_contract = is_path_in_dir(&base_path, src_dir, root_dir);
                    inherits_source_contract |= is_source_contract;
                    if base_path != full_path {
                        inherited_dependencies.insert(base_path);
                    }
                }
            }
            if inherits_source_contract {
                trace!("found mock contract {}", full_path.display());
                current_mocks.insert(full_path.clone());
            }
            if !inherited_dependencies.is_empty() {
                add_native_dependencies(
                    &mut current_native_dependencies,
                    full_path,
                    inherited_dependencies,
                );
            }
        }

        // Collect dependencies for non-mock test/script contracts.
        for (contract_id, contract, source, path) in candidate_contracts() {
            let full_path = normalize_path(&root_dir.join(path));

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
                contract_id,
                path,
                src_dir,
                root_dir,
                is_script,
                constructor_context.for_source(gcx, contract.source),
            );
            deps_collector.preserve_native_bytecode =
                native_return_data_contracts.contains(&contract_id);
            let mut using_dependencies = global_using_dependencies.clone();
            using_dependencies.extend(using_dependency_sources(
                gcx,
                source.usings.iter().chain(
                    contract
                        .linearized_bases
                        .iter()
                        .flat_map(|base_id| gcx.hir.contract(*base_id).usings),
                ),
            ));
            for source_id in using_dependencies {
                deps_collector.collect_source_dependencies(source_id);
            }
            // Analyze current contract.
            let _ = deps_collector.walk_contract(contract);
            if deps_collector.has_unresolved_native_dependency {
                conservative_files.insert(full_path.clone());
            }
            if !deps_collector.native_dependencies.is_empty() {
                add_native_dependencies(
                    &mut current_native_dependencies,
                    full_path.clone(),
                    deps_collector.native_dependencies,
                );
            }
            deps_collector.dependencies.retain(|dependency| {
                let dependency_id = dependency.referenced_contract;
                let dependency = gcx.hir.contract(dependency_id);
                let dependency_source = gcx.hir.source(dependency.source);
                let FileName::Real(dependency_path) = &dependency_source.file.name else {
                    conservative_files.insert(full_path.clone());
                    return false;
                };
                let has_constructor_args = dependency
                    .ctor
                    .is_some_and(|ctor_id| !gcx.hir.function(ctor_id).parameters.is_empty());
                if can_rewrite(
                    dependency_path,
                    path,
                    root_dir,
                    source_units,
                    remappings,
                    has_constructor_args,
                    dependency_id,
                ) {
                    true
                } else {
                    add_native_dependencies(
                        &mut current_native_dependencies,
                        full_path.clone(),
                        [normalize_path(&root_dir.join(dependency_path))],
                    );
                    false
                }
            });
            // Ignore empty test contracts declared in source files with other contracts.
            if !deps_collector.dependencies.is_empty() {
                preprocessed_contracts.insert(contract_id, deps_collector.dependencies);
            }
        }

        for file in conservative_files {
            current_native_dependencies.insert(file, NativeDependencyState::Conservative);
        }

        // Replace classifications only for files examined in this compiler job. This clears stale
        // mocks after a file is refactored while preserving fallback state across narrower jobs.
        for file in candidate_files {
            let state = current_native_dependencies.remove(&file);
            if preprocessor_state.update(file.clone(), state) {
                mocks.remove(&file);
            }
        }
        mocks.extend(current_mocks);

        for dependencies in preprocessed_contracts.values() {
            referenced_contracts.extend(dependencies.iter().map(|dep| dep.referenced_contract));
        }

        Self { preprocessed_contracts, referenced_contracts }
    }
}

/// Adds exact dependency paths unless the source is already classified conservatively.
fn add_native_dependencies(
    dependencies: &mut NativeDependencies,
    file: PathBuf,
    incoming: impl IntoIterator<Item = PathBuf>,
) {
    match dependencies.entry(file) {
        std::collections::btree_map::Entry::Vacant(entry) => {
            entry.insert(NativeDependencyState::Known(incoming.into_iter().collect()));
        }
        std::collections::btree_map::Entry::Occupied(entry) => {
            if let NativeDependencyState::Known(dependencies) = entry.into_mut() {
                dependencies.extend(incoming);
            }
        }
    }
}

/// Returns sources whose code can be embedded through the given `using for` directives.
fn using_dependency_sources<'gcx>(
    gcx: Gcx<'gcx>,
    directives: impl IntoIterator<Item = &'gcx UsingDirective<'gcx>>,
) -> HashSet<SourceId> {
    directives
        .into_iter()
        .flat_map(|directive| directive.entries)
        .flat_map(|entry| match entry.kind {
            UsingEntryKind::Library(contract_id) => vec![gcx.hir.contract(contract_id).source],
            UsingEntryKind::Functions(function_ids) => function_ids
                .iter()
                .map(|function_id| gcx.hir.function(*function_id).source)
                .collect(),
            UsingEntryKind::Err(_) => Vec::new(),
        })
        .collect()
}

/// Represents a bytecode dependency kind.
#[derive(Debug)]
enum BytecodeDependencyKind {
    /// `type(Contract).creationCode`
    CreationCode,
    /// `type(Contract).runtimeCode`.
    RuntimeCode,
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
    /// The original expression must reach Solc to preserve its validation.
    preserve_native: bool,
}

/// Walks over contract HIR and collects [`BytecodeDependency`]s and referenced contracts.
struct BytecodeDependencyCollector<'gcx, 'src> {
    /// Source map, used for determining contract item locations.
    gcx: Gcx<'gcx>,
    /// Contract whose lexically owned bytecode references may be rewritten.
    owner_contract: ContractId,
    /// Constructor validation context of the source being rewritten.
    constructor_context: ConstructorContext,
    /// Source path of the current contract.
    source_path: PathBuf,
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
    /// Whether bytecode references are being visited outside the owning contract's rewrite scope.
    preserve_native_bytecode: bool,
    /// Dependencies collected for current contract.
    dependencies: Vec<BytecodeDependency>,
    /// Dependencies that cannot be rewritten and remain embedded in the importer bytecode.
    native_dependencies: BTreeSet<PathBuf>,
    /// Whether a native dependency could not be assigned a stable filesystem identity.
    has_unresolved_native_dependency: bool,
    /// Functions followed while finding transitively embedded implementation code.
    visited_functions: HashSet<FunctionId>,
    /// Imported sources already classified as native dependencies.
    visited_sources: HashSet<SourceId>,
    /// Constants followed while finding embedded initializers, including aliases and cycles.
    visited_variables: HashSet<VariableId>,
}

impl<'gcx, 'src> BytecodeDependencyCollector<'gcx, 'src> {
    fn new(
        gcx: Gcx<'gcx>,
        owner_contract: ContractId,
        source_path: &Path,
        src_dir: &'src Path,
        root_dir: &'src Path,
        is_script: bool,
        constructor_context: ConstructorContext,
    ) -> Self {
        Self {
            gcx,
            owner_contract,
            constructor_context,
            source_path: normalize_path(&root_dir.join(source_path)),
            src_dir,
            root_dir,
            is_script,
            preserve_native_creation_code: false,
            preserve_native_bytecode: false,
            dependencies: vec![],
            native_dependencies: BTreeSet::new(),
            has_unresolved_native_dependency: false,
            visited_functions: HashSet::new(),
            visited_sources: HashSet::new(),
            visited_variables: HashSet::new(),
        }
    }

    /// Visits embedded implementation code without allowing edits outside the owning contract.
    fn collect_function_dependency(&mut self, function_id: FunctionId) {
        let function = self.gcx.hir.function(function_id);
        if function.contract == Some(self.owner_contract)
            || !self.visited_functions.insert(function_id)
        {
            return;
        }

        let source = self.gcx.hir.source(function.source);
        if let FileName::Real(path) = &source.file.name {
            let path = normalize_path(&self.root_dir.join(path));
            if path != self.source_path {
                self.native_dependencies.insert(path);
            }
        } else {
            self.has_unresolved_native_dependency = true;
        }

        let previous = self.preserve_native_bytecode;
        self.preserve_native_bytecode = true;
        let _ = self.visit_function(function);
        self.preserve_native_bytecode = previous;
    }

    /// Visits an expression for dependencies without rewriting within its source range.
    fn collect_native_expr(&mut self, expr: &'gcx Expr<'gcx>) {
        let previous = self.preserve_native_bytecode;
        self.preserve_native_bytecode = true;
        let _ = self.visit_expr(expr);
        self.preserve_native_bytecode = previous;
    }

    /// Records a source containing embedded code and all of its transitive imports.
    fn collect_source_dependencies(&mut self, source_id: SourceId) {
        if !self.visited_sources.insert(source_id) {
            return;
        }
        let source = self.gcx.hir.source(source_id);
        if let FileName::Real(path) = &source.file.name {
            let path = normalize_path(&self.root_dir.join(path));
            if path != self.source_path {
                self.native_dependencies.insert(path);
            }
        } else {
            self.has_unresolved_native_dependency = true;
        }
        for &(_, imported_source) in source.imports {
            self.collect_source_dependencies(imported_source);
        }
    }

    /// Classifies a bytecode dependency as rewritable or native.
    fn collect_dependency(&mut self, dependency: BytecodeDependency) {
        let contract = self.gcx.hir.contract(dependency.referenced_contract);
        let source = self.gcx.hir.source(contract.source);
        let FileName::Real(path) = &source.file.name else {
            self.has_unresolved_native_dependency = true;
            return;
        };
        let native_path = normalize_path(&self.root_dir.join(path));

        if self.preserve_native_bytecode || dependency.preserve_native {
            self.native_dependencies.insert(native_path);
            return;
        }

        if matches!(&dependency.kind, BytecodeDependencyKind::RuntimeCode) {
            self.native_dependencies.insert(native_path);
            return;
        }

        // Script bytecode references must not be rewritten. See field doc on `is_script`.
        if self.is_script {
            match &dependency.kind {
                BytecodeDependencyKind::CreationCode | BytecodeDependencyKind::RuntimeCode => {
                    trace!("skip creationCode in script");
                    self.native_dependencies.insert(native_path);
                    return;
                }
                BytecodeDependencyKind::New { .. } => {
                    trace!("skip new-expression in script");
                    self.native_dependencies.insert(native_path);
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
            self.native_dependencies.insert(native_path);
            return;
        }

        let has_constructor_args = contract
            .ctor
            .is_some_and(|ctor_id| !self.gcx.hir.function(ctor_id).parameters.is_empty());
        // Solidity only permits a custom layout on the most-derived contract, so the generated
        // constructor helper cannot inherit a target that declares one; keep this dependency
        // native.
        if contract.layout.is_some() && has_constructor_args {
            trace!("skip dependency on custom-layout contract");
            self.native_dependencies.insert(native_path);
            return;
        }

        // Constructor parameter types are copied into a derived helper contract. Private
        // constants used as array dimensions are not accessible in that scope.
        if constructor_uses_private_constants(self.gcx, contract) {
            self.native_dependencies.insert(native_path);
            return;
        }

        // Remapped imports can have absolute or symlinked paths, while compiler input paths are
        // relative and configured source directories can be canonicalized.
        if !is_path_in_dir(path, self.src_dir, self.root_dir) {
            let path = path.display();
            trace!("keep external dependency {path} native");
            self.native_dependencies.insert(native_path);
            return;
        }

        self.dependencies.push(dependency);
    }

    /// Follows constants whose initializer can embed code from another source.
    fn collect_variable_dependency(&mut self, id: VariableId) {
        let variable = self.gcx.hir.variable(id);
        if !variable.is_constant() || !self.visited_variables.insert(id) {
            return;
        }
        if let FileName::Real(path) = &self.gcx.hir.source(variable.source).file.name {
            let path = normalize_path(&self.root_dir.join(path));
            if path != self.source_path {
                self.native_dependencies.insert(path);
            }
        } else {
            self.has_unresolved_native_dependency = true;
        }
        if let Some(initializer) = variable.initializer {
            self.collect_native_expr(initializer);
        }
    }
}

/// Returns whether constructor parameter types reference private constants.
fn constructor_uses_private_constants(gcx: Gcx<'_>, contract: &Contract<'_>) -> bool {
    contract.ctor.is_some_and(|ctor| {
        gcx.hir.function(ctor).parameters.iter().any(|&param| {
            gcx.hir
                .variable(param)
                .ty
                .visit(&gcx.hir, &mut |ty| {
                    if let TypeKind::Array(array) = &ty.kind
                        && let Some(size) = array.size
                    {
                        size.visit(&mut |expr| {
                            if gcx.resolved_variable(expr).is_some_and(|var| {
                                gcx.hir.variable(var).visibility == Some(Visibility::Private)
                            }) {
                                return ControlFlow::Break(());
                            }
                            ControlFlow::Continue(())
                        })?;
                    }
                    ControlFlow::Continue(())
                })
                .is_break()
        })
    })
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

    let helper_path = deploy_helper_path(contract_id.index(), source_units);
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

    fn visit_var(&mut self, var: &'gcx Variable<'gcx>) -> ControlFlow<Self::BreakValue> {
        let previous = self.preserve_native_creation_code;
        self.preserve_native_creation_code |= var.is_constant();
        self.walk_var(var)?;
        self.preserve_native_creation_code = previous;
        ControlFlow::Continue(())
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        #[allow(clippy::collapsible_match)]
        match &expr.kind {
            ExprKind::Ident(resolutions) => {
                for &resolution in *resolutions {
                    match resolution {
                        Res::Namespace(source_id) => {
                            self.collect_source_dependencies(source_id);
                        }
                        Res::Item(item) => {
                            if let Some(function_id) = item.as_function() {
                                self.collect_function_dependency(function_id);
                            } else if let Some(variable_id) = item.as_variable() {
                                self.collect_variable_dependency(variable_id);
                            }
                        }
                        _ => {}
                    }
                }
            }
            ExprKind::Call(call_expr, call_args, named_args) => {
                if let Some(dependency) = handle_call_expr(
                    self.gcx,
                    self.constructor_context,
                    expr,
                    call_expr,
                    call_args,
                    named_args,
                ) {
                    self.collect_dependency(dependency);
                    // Call options are copied into the replacement expression. Keep their
                    // bytecode references native so edits cannot overlap the outer replacement.
                    self.visit_expr(call_expr)?;
                    if let Some(call_options) = named_args {
                        for arg in call_options.args {
                            self.collect_native_expr(&arg.value);
                        }
                    }
                    self.visit_call_args(call_args)?;
                    return ControlFlow::Continue(());
                }
                if let Some(function_id) = self.gcx.resolved_function(call_expr) {
                    self.collect_function_dependency(function_id);
                }
            }
            ExprKind::Member(member_expr, ident) => {
                // Solar does not resolve `Library.function` member expressions as functions. The
                // internal library implementation is embedded in the caller, so classify the
                // library source and its imports as native dependencies directly.
                if let ExprKind::Ident(resolutions) = member_expr.kind {
                    for resolution in resolutions {
                        if let Res::Item(item) = resolution
                            && let Some(contract_id) = item.as_contract()
                        {
                            let contract = self.gcx.hir.contract(contract_id);
                            if contract.kind == ContractKind::Library {
                                self.collect_source_dependencies(contract.source);
                            }
                        }
                    }
                }
                if let ExprKind::TypeCall(ty) = &member_expr.kind
                    && let TypeKind::Custom(contract_id) = &ty.kind
                    && let Some(contract_id) = contract_id.as_contract()
                    && let kind = match ident.name.as_str() {
                        "creationCode" => BytecodeDependencyKind::CreationCode,
                        "runtimeCode" => BytecodeDependencyKind::RuntimeCode,
                        _ => return self.walk_expr(expr),
                    }
                {
                    self.collect_dependency(BytecodeDependency {
                        kind,
                        loc: span_to_range(self.gcx.sess.source_map(), expr.span),
                        referenced_contract: contract_id,
                        preserve_native: false,
                    });
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }

    fn visit_stmt(&mut self, stmt: &'gcx Stmt<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let StmtKind::Try(stmt_try) = stmt.kind
            && let ExprKind::Call(call_expr, ..) = &stmt_try.expr.kind
            && matches!(call_expr.kind, ExprKind::New(_))
        {
            // Keep try deployments native: a static-context violation halts the current frame,
            // whereas a deployment cheatcode revert could be caught by an untyped try. Typed
            // returns also require native creation to preserve the constructor catch boundary.
            self.collect_native_expr(&stmt_try.expr);

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
    gcx: Gcx<'_>,
    context: ConstructorContext,
    parent_expr: &Expr<'_>,
    call_expr: &Expr<'_>,
    call_args: &CallArgs<'_>,
    call_options: &Option<&CallOptions<'_>>,
) -> Option<BytecodeDependency> {
    if let ExprKind::New(ty_new) = &call_expr.kind
        && let TypeKind::Custom(item_id) = ty_new.kind
        && let Some(contract_id) = item_id.as_contract()
    {
        let source_map = gcx.sess.source_map();
        let name = source_map.span_to_snippet(ty_new.span).ok()?;

        // Calculate the offset to remove call options and parentheses between the new type and
        // constructor arguments. For example, in `new Counter {value: 333} (address(this))`, the
        // offset is used to replace `{value: 333} (` with `(`. This also removes closing
        // parentheses around the callee when no call options are present.
        let call_args_offset = if call_args.is_empty() {
            0
        } else {
            (call_args.span.lo() - ty_new.span.hi()).to_usize()
        };

        let args_len = parent_expr.span.hi() - ty_new.span.hi();
        return Some(BytecodeDependency {
            kind: BytecodeDependencyKind::New {
                name,
                args_length: args_len.to_usize(),
                call_args_offset,
                value: named_arg(call_options, "value", source_map),
                salt: named_arg(call_options, "salt", source_map),
            },
            // The HIR callee excludes parentheses, so start at the full call expression.
            loc: span_to_range(source_map, parent_expr.span.with_hi(call_expr.span.hi())),
            referenced_contract: contract_id,
            preserve_native: !valid_constructor_call(
                gcx,
                context,
                contract_id,
                call_args,
                call_options,
            ),
        });
    }
    None
}

/// Helper function to extract value of a given named arg.
fn named_arg(
    call_options: &Option<&CallOptions<'_>>,
    arg: &str,
    source_map: &SourceMap,
) -> Option<String> {
    call_options
        .map(|options| options.args)
        .unwrap_or_default()
        .iter()
        .find(|named_arg| named_arg.name.as_str() == arg)
        .and_then(|named_arg| source_map.span_to_snippet(named_arg.value.span).ok())
}

/// Goes over all test/script files and replaces bytecode dependencies with cheatcode
/// invocations.
///
/// Try deployments remain native to preserve their constructor failure boundaries.
pub(crate) fn remove_bytecode_dependencies(
    gcx: Gcx<'_>,
    deps: &PreprocessorDependencies,
    data: &PreprocessorData,
) -> Updates {
    let mut updates = Updates::default();
    let reserved_identifiers = gcx
        .hir
        .source_ids()
        .map(|source_id| gcx.hir.source(source_id).file.src.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for (contract_id, deps) in &deps.preprocessed_contracts {
        let contract = gcx.hir.contract(*contract_id);
        let source = gcx.hir.source(contract.source);
        let FileName::Real(path) = &source.file.name else {
            continue;
        };

        let updates = updates.entry(path.clone()).or_default();
        let mut used_helpers = BTreeSet::new();

        let vm_interface_name = unique_identifier(
            &reserved_identifiers,
            format!("VmContractHelper{}", contract_id.index()),
        );
        // `address(uint160(uint256(keccak256("hevm cheat code"))))`
        let vm = format!("{vm_interface_name}(0x7109709ECfa91a80626fF3989D68f67F5b1DD12D)");
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
                BytecodeDependencyKind::RuntimeCode => {
                    unreachable!("runtimeCode is never rewritten")
                }
                BytecodeDependencyKind::New {
                    name,
                    args_length,
                    call_args_offset,
                    value,
                    salt,
                } => {
                    let mut update = format!("{name}(payable(");
                    let closing_seq = "})))";
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

                    if let Some(constructor_data) = constructor_data {
                        // Insert our helper.
                        used_helpers.insert(dep.referenced_contract);
                        let helper_contract = unique_identifier(
                            &reserved_identifiers,
                            constructor_data.helper_contract.clone(),
                        );
                        let encode_function = unique_identifier(
                            &reserved_identifiers,
                            constructor_data.encode_function.clone(),
                        );

                        update.push_str(", ");
                        update.push_str(&format!(
                            "_args: {}({}.{}",
                            encode_function, helper_contract, constructor_data.args_struct,
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

        let helper_imports = used_helpers
            .into_iter()
            .map(|id| {
                let constructor = data[&id].constructor_data.as_ref().unwrap();
                let helper_contract = &constructor.helper_contract;
                let encode_function = &constructor.encode_function;
                let local_helper =
                    unique_identifier(&reserved_identifiers, helper_contract.clone());
                let local_encoder =
                    unique_identifier(&reserved_identifiers, encode_function.clone());
                let helper_import = import_alias(helper_contract, &local_helper);
                let encoder_import = import_alias(encode_function, &local_encoder);
                let helper_path = constructor.helper_path.to_slash_lossy();
                format!("import {{{helper_import}, {encoder_import}}} from \"{helper_path}\";",)
            })
            .join("\n");
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

/// Returns an identifier that cannot collide with text in the original source.
fn unique_identifier(source: &str, mut identifier: String) -> String {
    while source.contains(&identifier) {
        identifier.push('_');
    }
    identifier
}

fn import_alias(identifier: &str, local: &str) -> String {
    if identifier == local { identifier.to_string() } else { format!("{identifier} as {local}") }
}

/// Checks constraints that disappear when `new` is replaced by a cheatcode call.
/// The generated argument struct retains type checks, but not source ABI-coder restrictions.
/// Keep parameterized ABI-coder-v1 calls native rather than duplicating Solc ABI validation.
fn valid_constructor_call(
    gcx: Gcx<'_>,
    context: ConstructorContext,
    id: ContractId,
    args: &CallArgs<'_>,
    options: &Option<&CallOptions<'_>>,
) -> bool {
    let contract = gcx.hir.contract(id);
    if contract.kind != ContractKind::Contract {
        return false;
    }
    let constructor = contract.ctor.map(|id| gcx.hir.function(id));
    let parameters = constructor.map_or(&[][..], |ctor| ctor.parameters);
    if args.len() != parameters.len() || (!parameters.is_empty() && !context.abi_coder_v2) {
        return false;
    }
    if let CallArgsKind::Named(args) = args.kind {
        let mut names = HashSet::new();
        if args.iter().any(|arg| {
            !names.insert(arg.name.name)
                || !parameters.iter().any(|id| {
                    gcx.hir.variable(*id).name.is_some_and(|name| name.name == arg.name.name)
                })
        }) {
            return false;
        }
    }
    if let Some(options) = options {
        let mut names = HashSet::new();
        if options.args.iter().any(|arg| {
            !names.insert(arg.name.name)
                || match arg.name.as_str() {
                    "salt" => !context.supports_create2,
                    "value" => constructor
                        .is_none_or(|ctor| ctor.state_mutability != StateMutability::Payable),
                    _ => true,
                }
        }) {
            return false;
        }
    }
    true
}

/// Finds return-buffer observations in a contract and the helpers it can call internally.
struct ReturnDataObserver<'gcx> {
    gcx: Gcx<'gcx>,
    observes_return_data: bool,
    functions: HashSet<FunctionId>,
    contracts: HashSet<ContractId>,
    visited_contracts: HashSet<ContractId>,
    member_functions: HashSet<(FunctionId, Symbol)>,
    sources: HashSet<SourceId>,
    operator_functions: HashSet<(FunctionId, UserDefinableOperator)>,
}

impl<'gcx> ReturnDataObserver<'gcx> {
    fn new(gcx: Gcx<'gcx>) -> Self {
        Self {
            gcx,
            observes_return_data: false,
            functions: HashSet::new(),
            contracts: HashSet::new(),
            visited_contracts: HashSet::new(),
            member_functions: HashSet::new(),
            sources: HashSet::new(),
            operator_functions: HashSet::new(),
        }
    }

    fn collect_member_functions(&mut self, source: SourceId, contract: Option<ContractId>) {
        let bases = contract
            .into_iter()
            .flat_map(|id| self.gcx.hir.contract(id).linearized_bases)
            .copied()
            .collect::<Vec<_>>();
        for &id in &bases {
            self.member_functions.extend(
                self.gcx
                    .hir
                    .contract(id)
                    .functions()
                    .filter_map(|id| self.gcx.hir.function(id).name.map(|name| (id, name.name))),
            );
        }
        let directives = self
            .gcx
            .hir
            .source(source)
            .usings
            .iter()
            .chain(bases.iter().flat_map(|&id| self.gcx.hir.contract(id).usings))
            .chain(
                self.gcx
                    .hir
                    .source_ids()
                    .flat_map(|id| self.gcx.hir.source(id).usings)
                    .filter(|directive| directive.global),
            );
        for directive in directives {
            for entry in directive.entries {
                match entry.kind {
                    UsingEntryKind::Library(id) => self.member_functions.extend(
                        self.gcx.hir.contract(id).functions().filter_map(|id| {
                            self.gcx.hir.function(id).name.map(|name| (id, name.name))
                        }),
                    ),
                    UsingEntryKind::Functions(ids) => {
                        if let Some(operator) = entry.operator {
                            self.operator_functions.extend(ids.iter().map(|&id| (id, operator)));
                        }
                        self.member_functions.extend(ids.iter().copied().filter_map(|id| {
                            entry
                                .name
                                .or_else(|| self.gcx.hir.function(id).name.map(|name| name.name))
                                .map(|name| (id, name))
                        }))
                    }
                    UsingEntryKind::Err(_) => {}
                }
            }
        }
    }
}

impl<'gcx> Visit<'gcx> for ReturnDataObserver<'gcx> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_nested_source(&mut self, id: SourceId) -> ControlFlow<Self::BreakValue> {
        if self.sources.insert(id) {
            let source = self.gcx.hir.source(id);
            self.walk_nested_source(id)?;
            for &(_, id) in source.imports {
                self.visit_nested_source(id)?;
            }
        }
        ControlFlow::Continue(())
    }

    fn visit_nested_contract(&mut self, id: ContractId) -> ControlFlow<Self::BreakValue> {
        if self.visited_contracts.insert(id) {
            let contract = self.gcx.hir.contract(id);
            self.contracts.insert(id);
            // State initializers run before functions are visited, but can already call methods
            // supplied by contract-scoped using directives.
            self.collect_member_functions(contract.source, Some(id));
            // An inherited observer can call a derived override that produces return data. Treat
            // the complete inheritance hierarchy as one execution scope before rewriting it.
            for &base in contract.linearized_bases {
                if base != id {
                    self.visit_nested_contract(base)?;
                }
            }
            self.walk_contract(contract)?;
        }
        ControlFlow::Continue(())
    }

    fn visit_nested_function(&mut self, id: FunctionId) -> ControlFlow<Self::BreakValue> {
        if self.functions.insert(id) {
            let function = self.gcx.hir.function(id);
            self.collect_member_functions(function.source, function.contract);
            if let Some(id) = function.contract {
                self.contracts.insert(id);
            }
            self.walk_function(function)?;
        }
        ControlFlow::Continue(())
    }

    fn visit_modifier(&mut self, modifier: &'gcx Modifier<'gcx>) -> ControlFlow<Self::BreakValue> {
        if let Some(id) = modifier.id.as_function() {
            self.visit_nested_function(id)?;
        }
        self.walk_modifier(modifier)
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
        let operator = match &expr.kind {
            ExprKind::Unary(op, _) => UserDefinableOperator::from_unop(op.kind),
            ExprKind::Binary(_, op, _) => UserDefinableOperator::from_binop(op.kind),
            _ => None,
        };
        if let Some(operator) = operator {
            // Type checking has not selected an overload yet, so visit every visible binding.
            let functions = self
                .operator_functions
                .iter()
                .filter_map(|&(id, bound)| (bound == operator).then_some(id))
                .collect::<Vec<_>>();
            for id in functions {
                self.visit_nested_function(id)?;
            }
        }
        match &expr.kind {
            ExprKind::Ident(resolutions) => {
                for resolution in *resolutions {
                    match resolution {
                        Res::Builtin(Builtin::YulReturndatasize | Builtin::YulReturndatacopy) => {
                            self.observes_return_data = true;
                        }
                        Res::Item(item) => {
                            if let Some(id) = item.as_function() {
                                self.visit_nested_function(id)?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            ExprKind::Call(callee, _, _) => {
                if let Some(id) = self.gcx.resolved_function(callee) {
                    self.visit_nested_function(id)?;
                }
            }
            ExprKind::Member(member, name) => {
                // Resolve the complete namespace path, including renamed re-exports and library
                // members, without visiting unrelated declarations in the imported sources.
                let mut names = vec![*name];
                let mut root = member.peel_parens();
                while let ExprKind::Member(parent, name) = &root.kind {
                    names.push(*name);
                    root = parent.peel_parens();
                }
                if let ExprKind::Ident(resolutions) = &root.kind {
                    names.reverse();
                    for resolution in *resolutions {
                        if let Res::Namespace(source) = resolution {
                            if let Some(resolutions) =
                                self.gcx.source_path_resolutions(&names, *source, None)
                                && let Some(targets) = resolutions.last()
                            {
                                for target in targets {
                                    if let Res::Item(item) = target {
                                        if let Some(id) = item.as_function() {
                                            self.visit_nested_function(id)?;
                                        } else if let Some(id) = item.as_variable() {
                                            self.visit_nested_var(id)?;
                                        }
                                    }
                                }
                            } else {
                                // Retain the conservative fallback when name resolution is
                                // incomplete.
                                self.visit_nested_source(*source)?;
                            }
                        }
                    }
                }
                // Type checking has not run, so include every visible overload of an inherited
                // or using-for method with this name.
                let functions = self
                    .member_functions
                    .iter()
                    .copied()
                    .filter_map(|(id, attached_name)| (attached_name == name.name).then_some(id))
                    .collect::<Vec<_>>();
                for id in functions {
                    self.visit_nested_function(id)?;
                }
                if let ExprKind::Ident(resolutions) = member.peel_parens().kind {
                    for resolution in resolutions {
                        if let Res::Item(item) = resolution
                            && let Some(id) = item.as_contract()
                            && self.gcx.hir.contract(id).kind == ContractKind::Library
                        {
                            for id in self.gcx.hir.contract(id).functions() {
                                if self
                                    .gcx
                                    .hir
                                    .function(id)
                                    .name
                                    .is_some_and(|ident| ident.name == name.name)
                                {
                                    self.visit_nested_function(id)?;
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        self.walk_expr(expr)
    }
}
