use clap::{Parser, ValueHint};
use foundry_cli::opts::BuildOpts;
use itertools::Itertools;
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

            let mut collector = Collector::new();
            _ = collector.visit_source_unit(&ast);

            let defs = collector.definitions;

            let resolver = Resolver::new(&defs);

            for id in defs.ordered.iter() {
                if let Some(resolved) = resolver.resolve_type_eip712(id) {
                    sh_println!("{resolved}\n");
                }
            }

            Ok(())
        });

        Ok(())
    }
}

//-------------------------------------------------------------------------
// 1. Collected Definitions
//-------------------------------------------------------------------------

/// Holds references to struct and enum definitions collected from the AST,
/// uniquely identified by "{contract_name}_{type_name}" or just "{type_name}"
/// for file-level definitions.
#[derive(Debug, Default)]
pub struct Definitions<'ast> {
    /// Maps unique identifier -> struct definition.
    pub structs: HashMap<String, &'ast ItemStruct<'ast>>,
    /// Maps unique identifier -> enum definition.
    pub enums: HashMap<String, &'ast ItemEnum<'ast>>,
    /// Maps unique identifier -> contract definition (useful for 'address' resolution).
    pub contracts: HashMap<String, &'ast ItemContract<'ast>>,
    /// Maps unique identifier -> contract definition (useful for 'address' resolution).
    pub ordered: Vec<String>,
}

//-------------------------------------------------------------------------
// 2. Definition Collector (AST Visitor)
//-------------------------------------------------------------------------

/// AST Visitor using `alloy-sol-type-parser::visit::Visit` to populate
/// `CollectedDefinitions`.
#[derive(Debug)]
pub struct Collector<'ast> {
    /// Reference to the collection being populated.
    pub definitions: Definitions<'ast>,
    /// The name of the contract currently being visited.
    current_contract_name: Option<&'ast str>,
}

impl<'ast> Collector<'ast> {
    pub fn new() -> Self {
        Self { definitions: Definitions::default(), current_contract_name: None }
    }

    fn make_key(&self, item_name: &str) -> String {
        match &self.current_contract_name {
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
        let contract_name = contract.name.name.as_str();

        // Store contract definition itself
        self.definitions.contracts.insert(contract_name.to_owned(), contract);

        // Set ctx before for visiting children
        self.current_contract_name.replace(contract_name);
        self.walk_item_contract(contract)
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct<'ast>) -> ControlFlow<Self::BreakValue> {
        let struct_name = item.name.name.as_str();
        let key = self.make_key(struct_name);

        self.definitions.ordered.push(key.clone());
        self.definitions.structs.insert(key, item);

        self.walk_item_struct(item)
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum<'ast>) -> ControlFlow<Self::BreakValue> {
        let enum_name = item.name.name.as_str();
        let key = self.make_key(enum_name);

        self.definitions.enums.insert(key, item);

        self.walk_item_enum(item)
    }
}

//-------------------------------------------------------------------------
// 4. Resolution Context (Helper for Recursive Resolution)
//-------------------------------------------------------------------------

/// Holds state during the recursive resolution of a single root struct's EIP-712 type.
#[derive(Debug)]
struct ResolutionContext {
    /// Tracks structs encountered during resolution to generate subtype definitions
    /// and handle recursion/naming conflicts.
    /// Maps the name assigned *in the EIP-712 string* (e.g., "MyStruct", "MyStruct_1")
    /// back to the unique key of the struct definition ("ContractName_MyStruct").
    subtypes: BTreeMap<String, String>, // BTreeMap for deterministic output order
    /// The unique key ("{contract_name}_{struct_name}") of the struct currently being processed.
    /// Used to resolve unqualified relative type names.
    current_struct_key: String,
}

impl ResolutionContext {
    fn new(root_struct_key: String) -> Self {
        Self { subtypes: BTreeMap::new(), current_struct_key: root_struct_key }
    }

    /// Gets the contract name part from the current struct key, if present.
    fn current_contract(&self) -> Option<&str> {
        self.current_struct_key.split('.').next()
    }

    /// Tries to find the assigned EIP-712 name for a given unique definition key.
    fn get_assigned_name<'a>(&'a self, definition_key: &str) -> Option<&'a str> {
        self.subtypes.iter().find_map(|(assigned_name, key)| {
            if key == definition_key {
                Some(assigned_name.as_str())
            } else {
                None
            }
        })
    }

    /// Assigns a unique name for EIP-712 encoding based on the original name,
    /// handling potential conflicts (e.g., Struct vs Struct_1).
    /// Adds the mapping `assigned_name -> definition_key` to subtypes.
    fn assign_name(&mut self, base_name: &str, definition_key: String) -> String {
        let mut name = base_name.to_string();
        let mut i = 0;
        while self.subtypes.contains_key(&name) {
            if let Some(existing_key) = self.subtypes.get(&name) {
                if existing_key == &definition_key {
                    return name;
                }
            }

            // Name collision for different struct definition --> append index.
            i += 1;
            name = format!("{}_{}", base_name, i);
        }
        self.subtypes.insert(name.clone(), definition_key);
        name
    }
}

//-------------------------------------------------------------------------
// 3. EIP-712 Resolver
//-------------------------------------------------------------------------

/// Responsible for generating the EIP-712 `encodeType` string for a given struct.
#[derive(Debug)]
pub struct Resolver<'a, 'ast> {
    /// Reference to all collected definitions from the project.
    definitions: &'a Definitions<'ast>,
    // No need for resolved_simple_types cache here, resolution happens dynamically
}

impl<'a, 'ast> Resolver<'a, 'ast> {
    pub fn new(definitions: &'a Definitions<'ast>) -> Self {
        Self { definitions }
    }

    /// Generates the full EIP-712 `encodeType` string for a root struct,
    /// including all necessary subtype definitions.
    ///
    /// # Arguments
    /// * `root_struct_unique_key`: The unique identifier ("{contract_name}.{struct_name}" or
    ///   "{struct_name}") of the struct to encode.
    pub fn resolve_type_eip712(&self, root_struct_unique_key: &str) -> Option<String> {
        // Ensure the root struct actually exists
        let root_struct_def = self.definitions.structs.get(root_struct_unique_key)?;

        let mut context = ResolutionContext::new(root_struct_unique_key.to_string());
        let root_base_name = root_struct_def.name.as_str();

        // Assign name for the root struct itself and add to subtypes
        let root_assigned_name =
            context.assign_name(root_base_name, root_struct_unique_key.to_string());

        // Recursively resolve the root struct *first* to populate the context.subtypes fully.
        let root_struct_encoded_partial = self.resolve_struct_recursive(
            root_struct_unique_key,
            &root_assigned_name,
            &mut context,
        )?;

        // Now, build the final string including all collected subtypes
        let mut final_result = root_struct_encoded_partial;

        // Iterate over subtypes collected during the root struct resolution.
        // The BTreeMap ensures deterministic order.
        let subtype_keys: Vec<(String, String)> = context.subtypes.clone().into_iter().collect();
        for (assigned_name, definition_key) in subtype_keys {
            // Skip the root struct itself as we already have its definition
            if definition_key == root_struct_unique_key {
                continue;
            }

            // Resolve each subtype
            let subtype_encoded =
                self.resolve_struct_recursive(&definition_key, &assigned_name, &mut context)?;
            final_result.push_str(&subtype_encoded);
        }

        Some(final_result)
    }

    /// Recursively generates the EIP-712 definition string for a *single* struct.
    /// Populates `context.subtypes` if nested structs are encountered.
    /// Does NOT append subtype definitions here; that happens in the main function.
    fn resolve_struct_recursive(
        &self,
        struct_key: &str,
        assigned_name: &str, // The name this struct should have in the output string
        context: &mut ResolutionContext,
    ) -> Option<String> {
        let struct_def = self.definitions.structs.get(struct_key)?;

        // Update context for resolving types *within* this struct
        let old_key = std::mem::replace(&mut context.current_struct_key, struct_key.to_string());

        let mut result = format!("{}(", assigned_name);
        let num_fields = struct_def.fields.len();

        for (idx, field) in struct_def.fields.iter().enumerate() {
            let field_name = field.name.as_ref()?;

            // Resolve the type of the field
            let type_string = match self.resolve_type(&field.ty, context) {
                Some(ty) => ty,
                None => {
                    // Unsupported type encountered in this struct
                    context.current_struct_key = old_key; // Restore context before returning
                    return None;
                }
            };

            _ = write!(result, "{} {}", type_string, field_name);

            if idx < num_fields - 1 {
                result.push(',');
            }
        }
        result.push(')');

        // Restore context
        context.current_struct_key = old_key;

        Some(result)
    }

    /// Resolves an `ast::Type` into an EIP-712 compatible type string.
    /// Populates `context.subtypes` if new user-defined struct types are encountered.
    fn resolve_type(
        &self,
        ty: &'ast Type<'ast>,
        context: &mut ResolutionContext,
    ) -> Option<String> {
        match &ty.kind {
            TypeKind::Elementary(name) => Some(name.to_abi_str().to_string()),
            TypeKind::Array(arr) => {
                let Some(inner_type) = self.resolve_type(&arr.element, context) else {
                    return None
                };
                let size_str = parse_array_size(&arr.size);
                Some(format!("{}[{}]", inner_type, size_str.unwrap_or(String::new())))
            }
            TypeKind::Custom(ast_path) => {
                let type_name = ast_path.segments().iter().map(|s| s.name.as_str()).join(".");

                // 1. Try resolving relative to the current contract scope
                let potential_key = if ast_path.segments().len() > 1 {
                    // If type_name_from_ast is "Structs.Foo", it's already fully qualified.
                    type_name.clone()
                } else {
                    context
                        .current_contract()
                        .map(|contract_name| format!("{}.{}", contract_name, type_name))
                        .unwrap_or_else(|| type_name.to_string())
                };

                if let Some(struct_def) = self.definitions.structs.get(&potential_key) {
                    // Check if we already assigned a name for this specific struct definition
                    if let Some(assigned_name) = context.get_assigned_name(&potential_key) {
                        return Some(assigned_name.to_string());
                    } else {
                        // New subtype encountered during this resolution. Assign a name.
                        let base_name = struct_def.name.as_str();
                        let assigned_name = context.assign_name(base_name, potential_key.clone());
                        // This call modifies `context` by adding deeper nested types if any.
                        _ = self.resolve_struct_recursive(&potential_key, &assigned_name, context);

                        // Return the assigned name for the current field's type resolution.
                        return Some(assigned_name);
                    }
                }

                if self.definitions.enums.contains_key(&potential_key) {
                    // EIP-712 typically encodes enums as uint8
                    return Some("uint8".to_string());
                }

                if self.definitions.contracts.contains_key(&potential_key) {
                    return Some("address".to_string());
                }

                println!(
                    "[WARN] Could not resolve custom type: {} (tried key: {})",
                    type_name, potential_key
                );
                None
            }
            TypeKind::Mapping(_) | TypeKind::Function { .. } => {
                println!("[WARN] Unsupported type: Function or Mapping");
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
