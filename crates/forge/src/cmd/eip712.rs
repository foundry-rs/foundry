use clap::{Parser, ValueHint};
use foundry_cli::opts::BuildOpts;
use solar_parse::{
    ast::{
        self, visit::Visit, Arena, Expr, ExprKind, Ident, ItemStruct, LitKind, SourceUnit, Span,
        Symbol, Type, TypeKind,
    },
    interface::{data_structures::Never, diagnostics, Session},
};
use std::{borrow::Cow, collections::HashMap, fmt::Write, ops::ControlFlow, path::PathBuf};

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
            let resolver = Resolver::new(&ast);

            let mut collector = StructCollector::default();
            _ = collector.visit_source_unit(&ast);

            let structs_in_target = collector.0.by_id;
            for id in structs_in_target.keys() {
                if let Some(resolved) = resolver.resolve_struct_eip712(*id) {
                    sh_println!("resolved:\n{resolved}\n\n");
                }
            }

            Ok(())
        });

        Ok(())
    }
}

/// AST [Visitor] used for collecting struct definitions.
#[derive(Debug, Default)]
pub struct StructCollector<'ast>(pub StructArena<'ast>);

impl<'ast> Visit<'ast> for StructCollector<'ast> {
    type BreakValue = Never;

    fn visit_item_struct(
        &mut self,
        strukt: &'ast ast::ItemStruct<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        self.0.insert(strukt);
        self.walk_item_struct(strukt)
    }
}

#[derive(Debug, Default)]
pub struct StructArena<'ast> {
    pub items: Vec<&'ast ItemStruct<'ast>>,
    pub by_id: HashMap<Span, usize>,
}

impl<'ast> StructArena<'ast> {
    pub fn insert(&mut self, item: &'ast ItemStruct<'ast>) {
        let idx = self.items.len();
        self.by_id.insert(item.name.span, idx);
        self.items.push(item);
    }

    pub fn get(&self, symbol: &Span) -> Option<&'ast ItemStruct<'ast>> {
        self.by_id.get(symbol).and_then(|idx| self.items.get(*idx).copied())
    }
}

impl<'ast> From<StructCollector<'ast>> for StructArena<'ast> {
    fn from(value: StructCollector<'ast>) -> Self {
        let Self { items, by_id } = value.0;
        Self { items, by_id }
    }
}

/// Collects mapping from AST id of type definition to representation of this type for EIP-712
/// encoding.
///
/// For now, maps contract definitions to `address` and enums to `uint8`.
#[derive(Debug, Clone, Default)]
struct SimpleCustomTypesCollector<'ast>(HashMap<Span, Cow<'ast, str>>);

impl<'ast> Visit<'ast> for SimpleCustomTypesCollector<'ast> {
    type BreakValue = Never;

    fn visit_item_contract(
        &mut self,
        contract: &'ast ast::ItemContract<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        self.0.insert(contract.name.span, Cow::Borrowed("address"));
        self.walk_item_contract(contract)
    }

    fn visit_item_enum(
        &mut self,
        enum_: &'ast ast::ItemEnum<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        self.0.insert(enum_.name.span, Cow::Borrowed("uint8"));
        self.walk_item_enum(enum_)
    }
}

#[derive(Debug)]
pub struct Resolver<'ast> {
    simple_types: HashMap<Span, Cow<'ast, str>>,
    structs: StructArena<'ast>,
}

impl<'ast> Resolver<'ast> {
    pub fn new(ast: &'ast SourceUnit<'ast>) -> Self {
        let simple_types = {
            let mut collector = SimpleCustomTypesCollector::default();
            _ = collector.visit_source_unit(ast);
            collector.0
        };

        let structs: StructArena<'ast> = {
            let mut collector = StructCollector::default();
            _ = collector.visit_source_unit(ast);
            collector.into()
        };

        println!("[DEBUG STRUCTS] {:#?}", &structs.items);

        Self { simple_types, structs }
    }

    /// Converts a given struct definition into EIP-712 `encodeType` representation.
    ///
    /// Returns `None` if struct contains any fields that are not supported by EIP-712 (e.g.
    /// mappings or function pointers).
    pub fn resolve_struct_eip712(&self, id: Span) -> Option<String> {
        let mut subtypes = StructArena::default();
        let primary = self.structs.get(&id)?;
        subtypes.insert(primary);

        self.resolve_eip712_inner(id, &mut subtypes, true, None)
    }

    fn resolve_eip712_inner(
        &self,
        id: Span, // This 'id' is the Symbol of the struct to format *in this call*
        subtypes: &mut StructArena<'ast>,
        append_subtypes: bool, // True only for the top-level call
        rename: Option<&str>,
    ) -> Option<String> {
        // Get the AST node for the struct 'id'
        let ast = self.structs.get(&id)?; // Use ? for conciseness if errors are handled upstream or via Option

        let struct_name = rename.unwrap_or_else(|| ast.name.as_str());
        // Use a temporary buffer for the struct signature part
        let mut struct_sig = format!("{struct_name}(");
        let mut first_field = true;

        // Process fields of the current struct 'id'
        for member in ast.fields.iter() {
            // Pass 'subtypes' mutably so resolve_type can add dependencies
            let ty_str = self.resolve_type(&member.ty, subtypes)?; // Use ?

            let field_name = member.name?; // Use ?

            if !first_field {
                struct_sig.push(',');
            }
            // Use write! macro, check its result if necessary (it returns std::fmt::Result)
            let _ = write!(struct_sig, "{ty_str} {}", field_name.as_str()); // Or handle error
            first_field = false;
        }
        struct_sig.push(')');

        // Now, handle appending subtypes if needed
        if append_subtypes {
            let mut result = struct_sig; // Start with the primary type signature

            // --- Sorting Logic (if needed) ---
            // 1. Collect subtype IDs (excluding the primary 'id')
            let subtype_ids: Vec<Span> =
                subtypes.by_id.keys().cloned().filter(|&subtype_id| subtype_id != id).collect();

            // 2. Get struct names for sorting
            let mut subtypes_to_append: Vec<(String, Span)> = subtype_ids
                .into_iter()
                .filter_map(|subtype_id| {
                    self.structs
                        .get(&subtype_id)
                        .map(|item| (item.name.as_str().to_string(), subtype_id))
                })
                .collect();

            // 3. Sort by struct name (alphabetically)
            subtypes_to_append.sort_by(|(name_a, _), (name_b, _)| name_a.cmp(name_b));
            // --- End Sorting Logic ---

            // Alternative: Use the original order if sorting isn't required
            // let collected_ids: Vec<Symbol> = subtypes.by_id.keys().cloned().filter(|&sid| sid !=
            // id).collect();

            // Iterate through the (potentially sorted) subtype IDs
            for (_, subtype_id) in subtypes_to_append {
                // Or iterate `collected_ids` if not sorting
                // Recursively call *this* function, but prevent it from appending *its* subtypes
                // again. Pass the current `subtypes` arena along.
                match self.resolve_eip712_inner(subtype_id, subtypes, false, None) {
                    // append_subtypes = false
                    Some(subtype_str) => {
                        result.push('\n');
                        result.push_str(&subtype_str);
                    }
                    None => {
                        eprintln!(
                            "Failed to resolve subtype definition for '{}' during EIP-712 generation.",
                            self.structs.get(&subtype_id).map(|s| s.name.as_str()).unwrap_or("unknown")
                        );
                        return None; // Fail the whole process if a dependency fails
                    }
                }
            }
            Some(result) // Return the combined string
        } else {
            Some(struct_sig) // Just return the signature if not appending subtypes
        }
    }

    /// Converts given [TypeName] into a type which can be converted to
    /// [`alloy_dyn_abi::DynSolType`].
    ///
    /// Returns `None` if the type is not supported for EIP712 encoding.
    pub fn resolve_type(
        &self,
        ty: &Type<'ast>,
        subtypes: &mut StructArena<'ast>,
    ) -> Option<Cow<'ast, str>> {
        match ty.kind {
            TypeKind::Function(_) | TypeKind::Mapping(_) => None,
            TypeKind::Elementary(ty) => Some(ty.to_abi_str()),
            TypeKind::Array(ref ty) => {
                let Some(inner) = self.resolve_type(&ty.element, subtypes) else { return None };
                let len = parse_array_size(&ty.size);

                Some(Cow::Owned(format!("{inner}[{}]", len.unwrap_or(String::new()))))
            }
            TypeKind::Custom(ref ast_path) => {
                // Flatten path to produce a unique symbol for each type
                let type_name = ast_path
                    .segments()
                    .iter()
                    .map(|ident| ident.name.as_str())
                    .collect::<Vec<_>>()
                    .concat();
                let type_symbol = Symbol::intern(type_name.as_str());
                println!("[DEBUG] CUSTOM TYPE\n - NAME: {}\n - SYMBOL: {}", type_name, type_symbol);

                // TODO: adjust
                let type_symbol = ast_path.last().span;

                // 1. Check simple types (contracts -> address, enums -> uint8)
                if let Some(resolved_simple) = self.simple_types.get(&type_symbol) {
                    return Some(resolved_simple.clone());
                }

                // 2. Check if it's a struct we know about
                if let Some(struct_def) = self.structs.get(&type_symbol) {
                    // It's a struct. Add it to subtypes if not already present.
                    // Use the struct's name as the type name.
                    if subtypes.get(&type_symbol).is_none() {
                        subtypes.insert(struct_def);

                        // Recursively resolve types of members to populate `subtypes` further
                        // and to check if all member types are valid.
                        for member_field in struct_def.fields.iter() {
                            if self.resolve_type(&member_field.ty, subtypes).is_none() {
                                // If any member type is not resolvable, this struct type is also
                                // not.
                                return None;
                            }
                        }
                    }
                    // The type name in EIP-712 is the struct's name
                    return Some(Cow::Owned(type_name));
                }

                // 3. If not found, it might be an unsupported custom type or undefined
                // For now, treat as unsupported for EIP-712
                // Consider adding error handling/logging here
                eprintln!(
                    "Warning: Custom type '{:?}' not found or not supported for EIP-712.",
                    type_symbol
                );
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
