use super::{
    data::{ContractData, PreprocessorData},
    span_to_range,
};
use foundry_compilers::Updates;
use itertools::Itertools;
use solar::sema::{
    Gcx, Hir,
    hir::{CallArgs, ContractId, Expr, ExprKind, NamedArg, Stmt, StmtKind, TypeKind, Visit},
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
        src_dir: &Path,
        root_dir: &Path,
        mocks: &mut HashSet<PathBuf>,
    ) -> Self {
        let mut preprocessed_contracts = BTreeMap::new();
        let mut referenced_contracts = HashSet::new();
        let mut current_mocks = HashSet::new();

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
                    FileName::Real(base_path) if base_path.starts_with(src_dir)
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

            if current_mocks.contains(&full_path) {
                trace!("{} is a mock, skipping", path.display());
                continue;
            }

            // Make sure current contract is not in list of mocks (could happen when a contract
            // which used to be a mock is refactored to a non-mock implementation).
            mocks.remove(&full_path);

            let mut deps_collector =
                BytecodeDependencyCollector::new(gcx, source.file.src.as_str(), src_dir);
            // Analyze current contract.
            let _ = deps_collector.walk_contract(contract);
            // Ignore empty test contracts declared in source files with other contracts.
            if !deps_collector.dependencies.is_empty() {
                preprocessed_contracts.insert(contract_id, deps_collector.dependencies);
            }

            // Record collected referenced contract ids.
            referenced_contracts.extend(deps_collector.referenced_contracts);
        }

        // Add current mocks.
        mocks.extend(current_mocks);

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
    /// Dependencies collected for current contract.
    dependencies: Vec<BytecodeDependency>,
    /// Unique HIR ids of contracts referenced from current contract.
    referenced_contracts: HashSet<ContractId>,
}

impl<'gcx, 'src> BytecodeDependencyCollector<'gcx, 'src> {
    fn new(gcx: Gcx<'gcx>, src: &'src str, src_dir: &'src Path) -> Self {
        Self { gcx, src, src_dir, dependencies: vec![], referenced_contracts: HashSet::default() }
    }

    /// Collects reference identified as bytecode dependency of analyzed contract.
    /// Discards any reference that is not in project src directory (e.g. external
    /// libraries or mock contracts that extend source contracts).
    fn collect_dependency(&mut self, dependency: BytecodeDependency) {
        let contract = self.gcx.hir.contract(dependency.referenced_contract);
        let source = self.gcx.hir.source(contract.source);
        let FileName::Real(path) = &source.file.name else {
            return;
        };

        if !path.starts_with(self.src_dir) {
            let path = path.display();
            trace!("ignore dependency {path}");
            return;
        }

        self.referenced_contracts.insert(dependency.referenced_contract);
        self.dependencies.push(dependency);
    }
}

impl<'gcx> Visit<'gcx> for BytecodeDependencyCollector<'gcx, '_> {
    type BreakValue = Never;

    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'gcx Expr<'gcx>) -> ControlFlow<Self::BreakValue> {
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
    named_args: &Option<&[NamedArg<'_>]>,
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
        let call_args_offset = if named_args.is_some() && !call_args.is_empty() {
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
                value: named_arg(src, named_args, "value", source_map),
                salt: named_arg(src, named_args, "salt", source_map),
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
    named_args: &Option<&[NamedArg<'_>]>,
    arg: &str,
    source_map: &SourceMap,
) -> Option<String> {
    named_args.unwrap_or_default().iter().find(|named_arg| named_arg.name.as_str() == arg).map(
        |named_arg| {
            let named_arg_loc = span_to_range(source_map, named_arg.value.span);
            src[named_arg_loc].to_string()
        },
    )
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

        let vm_interface_name = format!("VmContractHelper{}", contract_id.get());
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
                            (format!("this.addressTo{name}{id}(", id = contract_id.get()), "}))")
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
                        // Insert our helper
                        used_helpers.insert(dep.referenced_contract);

                        update.push_str(", ");
                        update.push_str(&format!(
                            "_args: encodeArgs{id}(DeployHelper{id}.FoundryPpConstructorArgs",
                            id = dep.referenced_contract.get()
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
                        id = contract_id.get()
                    )
                })
                .collect::<String>();

            updates.insert((last_fn_range.end, last_fn_range.end, to_address_fns));
        }

        let helper_imports = used_helpers.into_iter().map(|id| {
            let id = id.get();
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
    function getCode(string memory _artifact) external returns (bytes memory);
}}"#
            ),
        ));
    }
    updates
}
