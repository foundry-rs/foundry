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

            for id in defs.ordered.iter() {
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
/// Each type is uniquely identified by `{contract}_{type}` or just `{type}` for file-level
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
    /// Generates a unique identifier (`{contract}.{type}`) combining the contract and the type.
    fn generate_key(&self, item_name: &str) -> String {
        match &self.target_contract {
            Some(contract_name) => format!("{}.{}", contract_name, item_name),
            None => item_name.to_string(),
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
            name = format!("{}_{}", name, i);
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
        let def = self.defs.structs.get(key)?;

        let mut ctx = ResolutionCtx::new(key.to_string());
        let name = def.name.as_str();

        // Assign name for the primary struct
        let eip712_name = ctx.assign_eip712_name(name, key.to_string());

        // Recursively resolve primary struct to populate the ctx with all the subtypes.
        let mut result = self.resolve_eip712_inner(key, &eip712_name, &mut ctx)?;

        // Encode the subtypes
        let subtype_keys: Vec<(String, String)> =
            ctx.subtypes.iter().map(|(n, k)| (n.clone(), k.clone())).collect();
        for (sub_name, sub_key) in subtype_keys {
            if sub_key == key {
                continue;
            }

            let subtype_encoded = self.resolve_eip712_inner(&sub_key, &sub_name, &mut ctx)?;
            result.push_str(&subtype_encoded);
        }

        Some(result)
    }

    /// Recursively generates the EIP-712 encoding for the input struct.
    /// Does NOT append subtype definitions here (that happens in the main function).
    fn resolve_eip712_inner(
        &self,
        key: &str,
        eip712_name: &str,
        ctx: &mut ResolutionCtx,
    ) -> Option<String> {
        let struct_def = self.defs.structs.get(key)?;

        // Update target contract ctx before resolving subtypes of this struct
        let old_key = std::mem::replace(&mut ctx.current_key, key.to_string());

        let mut result = format!("{}(", eip712_name);
        let num_fields = struct_def.fields.len();

        for (idx, field) in struct_def.fields.iter().enumerate() {
            let field_name = field.name.as_ref()?;

            // Resolve the type of the field
            let type_string = self.resolve_type(&field.ty, ctx)?;
            _ = write!(result, "{} {}", type_string, field_name);

            if idx < num_fields - 1 {
                result.push(',');
            }
        }
        result.push(')');

        // Restore target contract ctx
        ctx.current_key = old_key;

        Some(result)
    }

    /// Converts the given [`solar_parse::ast::Type`] into a type which can be converted to
    /// [`alloy_dyn_abi::DynSolType`].
    ///
    /// Returns `None` if the type is not supported for EIP-712 encoding.
    fn resolve_type(&self, ty: &'ast Type<'ast>, ctx: &mut ResolutionCtx) -> Option<String> {
        match &ty.kind {
            TypeKind::Elementary(name) => Some(name.to_abi_str().to_string()),
            TypeKind::Array(arr) => {
                let Some(inner_type) = self.resolve_type(&arr.element, ctx) else { return None };
                let size_str = parse_array_size(&arr.size);
                Some(format!("{}[{}]", inner_type, size_str.unwrap_or(String::new())))
            }
            TypeKind::Custom(ast_path) => {
                let segments: Vec<_> =
                    ast_path.segments().iter().map(|s| s.name.as_str()).collect();

                let key = if segments.len() > 1 {
                    segments.join(".")
                } else {
                    ctx.target_contract()
                        .map(|contract_name| format!("{}.{}", contract_name, segments[0]))
                        .unwrap_or_else(|| segments[0].to_string())
                };

                if let Some(def) = self.defs.structs.get(&key) {
                    // Check if the the struct has already been processed. If not, assign it a name
                    // and resolve it.
                    if let Some(name) = ctx.get_eip712_name(&key) {
                        return Some(name.to_string());
                    } else {
                        // Update target contract ctx before resolving subtypes of this struct
                        let prev_key = std::mem::replace(&mut ctx.current_key, key.clone());

                        let name = ctx.assign_eip712_name(def.name.as_str(), key.clone());
                        for field_in_new_struct in def.fields.iter() {
                            if self.resolve_type(&field_in_new_struct.ty, ctx).is_none() {
                                ctx.current_key = prev_key;
                                return None;
                            }
                        }

                        // Restore target contract ctx
                        ctx.current_key = prev_key;

                        return Some(name);
                    }
                }

                if self.defs.enums.contains(&key) {
                    // For now, map enum definitions to `uint8`
                    return Some("uint8".to_string());
                }

                if self.defs.contracts.contains(&key.as_str()) {
                    // For now, map contract definitions to `address`
                    return Some("address".to_string());
                }

                eprintln!("[WARN] Could not resolve custom type: {}", key);
                None
            }
            TypeKind::Mapping(_) | TypeKind::Function { .. } => {
                eprintln!("[WARN] Unsupported type: Function or Mapping");
                None
            }
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
