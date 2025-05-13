use clap::{Parser, ValueHint};
use foundry_cli::opts::BuildOpts;
use solar_parse::{
    ast::{
        visit::Visit, Arena, Expr, ExprKind, ItemContract, ItemEnum, ItemStruct, LitKind, Type,
        TypeKind,
    },
    interface::{data_structures::Never, diagnostics, Session},
};
use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write,
    ops::ControlFlow,
    path::PathBuf,
};

foundry_config::impl_figment_convert!(Eip712Args, build);

/// CLI arguments for `forge eip712`.
#[derive(Clone, Debug, Parser)]
pub struct Eip712Args {
    /// The path to the file from which to read struct definitions.
    #[arg(value_hint = ValueHint::FilePath, value_name = "PATH")]
    pub target_path: PathBuf,

    #[command(flatten)]
    build: BuildOpts,
}

impl Eip712Args {
    pub fn run(self) -> eyre::Result<()> {
        let target_path = dunce::canonicalize(self.target_path)?;

        let mut sess = Session::builder().with_stderr_emitter().build();
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);
        let arena = Arena::new();

        let _ = sess.enter(|| -> Result<(), diagnostics::ErrorGuaranteed> {
            // Initialize the parser and get the AST
            let mut parser = solar_parse::Parser::from_file(&sess, &arena, &target_path)?;
            let ast = parser.parse_file().map_err(|e| e.emit())?;

            let mut collector = Collector::default();
            _ = collector.visit_source_unit(&ast);

            let defs = collector.defs;

            let resolver = Resolver::new(&defs);

            for id in &defs.ordered {
                if let Some(resolved) = resolver.resolve_type_eip712(id) {
                    _ = sh_println!("{resolved}\n");
                }
            }

            Ok(())
        });

        Ok(())
    }
}

/// Holds references to custom type definitions in the AST.
///
/// Each type is uniquely identified by `{contract}.{type}` or just `{type}` for file-level
/// definitions.
#[derive(Debug, Default)]
pub struct Definitions<'ast> {
    /// Tracks unique identifiers for enums.
    pub enums: Vec<String>,
    /// Tracks unique identifiers for contracts.
    pub contracts: Vec<&'ast str>,
    /// Maps unique identifier -> struct definition.
    pub structs: HashMap<String, &'ast ItemStruct<'ast>>,

    // TODO: discuss if we should remove it --> only necessary if we want to keep the unit tests in
    // the same order as originally impl.
    /// Tracks parser visit order.
    pub ordered: Vec<String>,
}

/// AST Visitor (implements [`Visit`]) used for collecting custom type definitions.
#[derive(Debug, Default)]
pub struct Collector<'ast> {
    /// Custom type definitions.
    pub defs: Definitions<'ast>,
    /// Currently visited contract.
    target_contract: Option<&'ast str>,
}

impl<'ast> Collector<'ast> {
    /// Generates a unique identifier (`{contract}.{ty_name}`) combining the contract and the type.
    fn generate_key(&self, ty_name: &str) -> String {
        match &self.target_contract {
            Some(contract) => format!("{contract}.{ty_name}"),
            None => ty_name.to_string(),
        }
    }
}

impl<'ast> Visit<'ast> for Collector<'ast> {
    type BreakValue = Never;

    fn visit_item_contract(
        &mut self,
        contract: &'ast ItemContract<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        let name = contract.name.name.as_str();
        self.defs.contracts.push(name);

        // Set ctx before for visiting children
        self.target_contract.replace(name);
        self.walk_item_contract(contract)
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum<'ast>) -> ControlFlow<Self::BreakValue> {
        let name = item.name.name.as_str();
        let key = self.generate_key(name);
        self.defs.enums.push(key);

        self.walk_item_enum(item)
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct<'ast>) -> ControlFlow<Self::BreakValue> {
        let name = item.name.name.as_str();
        let key = self.generate_key(name);
        self.defs.ordered.push(key.clone());
        self.defs.structs.insert(key, item);

        self.walk_item_struct(item)
    }
}

/// Holds state during the EIP-712 resolution of a struct, and helps dealing with unique names.
#[derive(Debug)]
struct ResolutionCtx {
    /// Maps the EIP-712 type name (`Foo`, `Foo_1`) back to its unique key (`{contract}.{type}`).
    subtypes: BTreeMap<String, String>,
    /// The unique key of the struct being processed.
    current_key: String,
}

impl ResolutionCtx {
    fn new(primary_key: String) -> Self {
        Self { subtypes: BTreeMap::new(), current_key: primary_key }
    }

    /// Gets the contract name part from the current struct key, if present.
    fn target_contract(&self) -> Option<&str> {
        self.current_key.split('.').next()
    }

    /// Tries to find the EIP-712 name for a given unique key.
    fn get_eip712_name<'a>(&'a self, key: &str) -> Option<&'a str> {
        self.subtypes.iter().find_map(|(n, k)| if k == key { Some(n.as_str()) } else { None })
    }

    /// Assigns a unique EIP-712 name. Updates state.
    fn assign_eip712_name(&mut self, name: &str, key: String) -> String {
        let mut name = name.to_string();
        let mut i = 0;
        while self.subtypes.contains_key(&name) {
            if let Some(k) = self.subtypes.get(&name) {
                if k == &key {
                    return name;
                }
            }
            i += 1;
            name = format!("{name}_{i}");
        }
        self.subtypes.insert(name.clone(), key);
        name
    }
}

/// Generates the EIP-712 `encodeType` string for a given struct.
#[derive(Debug)]
pub struct Resolver<'a, 'ast> {
    defs: &'a Definitions<'ast>,
}

impl<'a, 'ast> Resolver<'a, 'ast> {
    pub fn new(defs: &'a Definitions<'ast>) -> Self {
        Self { defs }
    }

    /// Converts a given struct definition into EIP-712 `encodeType` representation.
    ///
    /// Returns `None` if struct contains any fields that are not supported by EIP-712 (e.g.
    /// mappings or function pointers).
    pub fn resolve_type_eip712(&self, key: &str) -> Option<String> {
        let mut ctx = ResolutionCtx::new(key.to_string());

        // Assign name to the primary struct
        let eip712_name =
            ctx.assign_eip712_name(self.defs.structs.get(key)?.name.as_str(), key.to_string());

        // Recursively resolve and assign names to all the secondary types
        self.resolve_subtype_names(key, &mut ctx);

        // Encode the primary struct
        let mut result = self.encode_struct(key, &eip712_name, &ctx)?;

        // Encode all the secondary types in alphabetical order
        let subtypes_to_encode: Vec<(&String, &String)> =
            ctx.subtypes.iter().filter(|(_, k)| **k != key).collect();
        for (subtype_eip712_name, subtype_key) in subtypes_to_encode {
            result.push_str(&self.encode_struct(subtype_key, subtype_eip712_name, &ctx)?);
        }

        Some(result)
    }

    /// Discovers and assigns names to all subtypes of a given struct key.
    fn resolve_subtype_names(&self, key: &str, ctx: &mut ResolutionCtx) {
        if let Some(def) = self.defs.structs.get(key) {
            // Update target contract ctx before resolving subtypes of this struct
            let old_key = ctx.current_key.clone();
            ctx.current_key = key.to_string();

            for field in def.fields.iter() {
                self.resolve_eip712_name(&field.ty, ctx);
            }

            // Restor target contract ctx
            ctx.current_key = old_key;
        }
    }

    /// Assigns a unique name to the input struct and its subtypes.
    fn resolve_eip712_name(&self, ty: &'ast Type<'ast>, ctx: &mut ResolutionCtx) {
        match &ty.kind {
            TypeKind::Array(arr) => self.resolve_eip712_name(&arr.element, ctx),
            TypeKind::Custom(ast_path) => {
                let segments: Vec<_> =
                    ast_path.segments().iter().map(|s| s.name.as_str()).collect();

                let key = if segments.len() == 1 {
                    ctx.target_contract()
                        .map(|contract| format!("{}.{}", contract, segments[0]))
                        .unwrap_or_else(|| segments[0].to_string())
                } else {
                    segments.join(".")
                };

                // If unnamed, assign a name and resolve subtypes
                if self.defs.structs.contains_key(&key) && ctx.get_eip712_name(&key).is_none() {
                    if let Some(def) = self.defs.structs.get(&key) {
                        ctx.assign_eip712_name(def.name.as_str(), key.clone());
                        self.resolve_subtype_names(&key, ctx);
                    }
                }
            }
            _ => (),
        }
    }

    /// Generates the EIP-712 string representation of the struct.
    ///
    /// Requires all subtypes to already be named in `ctx`.
    fn encode_struct(&self, key: &str, eip712_name: &str, ctx: &ResolutionCtx) -> Option<String> {
        let struct_def = self.defs.structs.get(key)?;
        let mut result = format!("{eip712_name}(");
        let num_fields = struct_def.fields.len();

        for (idx, field) in struct_def.fields.iter().enumerate() {
            // Resolve the type of the field
            let encoded_type = self.resolve_type(&field.ty, key, ctx)?;
            _ = write!(result, "{} {}", encoded_type, field.name.as_ref()?);

            if idx < num_fields - 1 {
                result.push(',');
            }
        }
        result.push(')');
        Some(result)
    }

    /// Converts the given [`solar_parse::ast::Type`] into a type which can be converted to
    /// [`alloy_dyn_abi::DynSolType`]. For structs, uses the cached EIP-712 name.
    ///
    /// Returns `None` if the type is not supported for EIP-712 encoding.
    fn resolve_type(&self, ty: &'ast Type<'ast>, key: &str, ctx: &ResolutionCtx) -> Option<String> {
        match &ty.kind {
            TypeKind::Elementary(name) => Some(name.to_abi_str().to_string()),
            TypeKind::Array(arr) => {
                let inner_type = self.resolve_type(&arr.element, key, ctx)?;
                let size_str = parse_array_size(&arr.size).unwrap_or_default();
                Some(format!("{inner_type}[{size_str}]"))
            }
            TypeKind::Custom(ast_path) => {
                let segments: Vec<_> =
                    ast_path.segments().iter().map(|s| s.name.as_str()).collect();

                // If single segment, craft subtype key as the contract and the subtype
                let sub_key = if segments.len() == 1 {
                    key.split('.')
                        .next()
                        .map(|contract| format!("{}.{}", contract, segments[0]))
                        .unwrap_or_else(|| segments[0].to_string())
                } else {
                    segments.join(".")
                };

                if let Some(name) = ctx.get_eip712_name(&sub_key) {
                    Some(name.to_string())
                } else if self.defs.enums.contains(&sub_key) {
                    Some("uint8".to_string())
                } else if self.defs.contracts.contains(&sub_key.as_str()) {
                    Some("address".to_string())
                } else {
                    None // Missing type definition (compiler should fail)
                }
            }
            // EIP-712 doesn't support functions and mappings
            TypeKind::Mapping(_) | TypeKind::Function { .. } => None,
        }
    }
}

fn parse_array_size<'ast>(expr: &Option<&mut Expr<'ast>>) -> Option<String> {
    if let Some(ref expr) = expr {
        if let ExprKind::Lit(ref lit, None) = &expr.kind {
            if let LitKind::Number(ref int) = &lit.kind {
                return Some(int.to_string());
            }
        }
    }

    None
}
