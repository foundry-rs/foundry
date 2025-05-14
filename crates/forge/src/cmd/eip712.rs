use clap::{Parser, ValueHint};
use eyre::{eyre, Result};
use foundry_cli::opts::{solar_pcx_from_build_opts, BuildOpts};
use solar_parse::interface::Session;
use solar_sema::{
    ast::LitKind,
    hir::{Expr, ExprKind, ItemId, StructId, Type, TypeKind},
    thread_local::ThreadLocal,
    Hir,
};
use std::{collections::BTreeMap, fmt::Write, path::PathBuf};

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
    pub fn run(self) -> Result<()> {
        let mut sess = Session::builder().with_stderr_emitter().build();
        sess.dcx = sess.dcx.set_flags(|flags| flags.track_diagnostics = false);

        let _ = sess.enter(|| -> Result<()> {
            // Set up the parsing context with the project paths and sources.
            let parsing_context =
                solar_pcx_from_build_opts(&sess, self.build, vec![self.target_path])?;

            // Parse and resolve
            let hir_arena = ThreadLocal::new();
            if let Some(gcx) =
                parsing_context.parse_and_lower(&hir_arena).map_err(|_| eyre!("error parsing"))?
            {
                let hir = &gcx.get().hir;

                let resolver = Resolver::new(hir);
                for id in &resolver.struct_ids() {
                    if let Some(resolved) = resolver.resolve_struct_eip712(*id) {
                        _ = sh_println!("{resolved}\n");
                    }
                }
            }

            Ok(())
        });

        Ok(())
    }
}

/// Generates the EIP-712 `encodeType` string for a given struct.
///
/// Requires a reference to the source HIR.
#[derive(Debug)]
pub struct Resolver<'hir> {
    hir: &'hir Hir<'hir>,
}

impl<'hir> Resolver<'hir> {
    /// Constructs a new [`Resolver`] for the supplied [`Hir`] instance.
    pub fn new(hir: &'hir Hir<'hir>) -> Self {
        Self { hir }
    }

    /// Returns the [`StructId`]s of every user-defined struct in source order.
    pub fn struct_ids(&self) -> Vec<StructId> {
        self.hir.strukt_ids().collect()
    }

    /// Converts a given struct into its EIP-712 `encodeType` representation.
    ///
    /// Returns `None` if the struct, or any of its fields, contains constructs
    /// not supported by EIP-712 (mappings, function types, errors, etc).
    pub fn resolve_struct_eip712(&self, id: StructId) -> Option<String> {
        let mut subtypes = BTreeMap::new();
        subtypes.insert(self.hir.strukt(id).name.as_str().into(), id);
        self.resolve_eip712_inner(id, &mut subtypes, true, None)
    }

    fn resolve_eip712_inner(
        &self,
        id: StructId,
        subtypes: &mut BTreeMap<String, StructId>,
        append_subtypes: bool,
        rename: Option<&str>,
    ) -> Option<String> {
        let def = self.hir.strukt(id);
        let mut result = format!("{}(", rename.unwrap_or(def.name.as_str()));

        for (idx, field_id) in def.fields.iter().enumerate() {
            let field = self.hir.variable(*field_id);
            let ty = self.resolve_type(&field.ty, subtypes)?;

            write!(result, "{ty} {name}", name = field.name?.as_str()).ok()?;

            if idx < def.fields.len() - 1 {
                result.push(',');
            }
        }

        result.push(')');

        if append_subtypes {
            for (subtype_name, subtype_id) in
                subtypes.iter().map(|(name, id)| (name.clone(), *id)).collect::<Vec<_>>()
            {
                if subtype_id == id {
                    continue
                }
                let encoded_subtype =
                    self.resolve_eip712_inner(subtype_id, subtypes, false, Some(&subtype_name))?;

                result.push_str(&encoded_subtype);
            }
        }

        Some(result)
    }

    fn resolve_type(
        &self,
        ty: &'hir Type<'hir>,
        subtypes: &mut BTreeMap<String, StructId>,
    ) -> Option<String> {
        match ty.kind {
            TypeKind::Elementary(ty) => Some(ty.to_abi_str().to_string()),
            TypeKind::Array(arr) => {
                let inner_type = self.resolve_type(&arr.element, subtypes)?;
                let size_str = arr.size.and_then(|expr| parse_array_size(expr)).unwrap_or_default();
                Some(format!("{inner_type}[{size_str}]"))
            }
            TypeKind::Custom(item_id) => {
                match item_id {
                    // For now, map enums to `uint8`
                    ItemId::Enum(_) => Some("uint8".into()),
                    // For now, map contracts to `address`
                    ItemId::Contract(_) => Some("address".into()),
                    // Resolve user-defined type alias to the original type
                    ItemId::Udvt(id) => self.resolve_type(&self.hir.udvt(id).ty, subtypes),
                    // Recursively resolve structs
                    ItemId::Struct(id) => {
                        let def = self.hir.strukt(id);
                        let name =
                            // If the struct was already resolved, use its previously assigned name
                            if let Some((name, _)) = subtypes.iter().find(|(_, cached_id)| id == **cached_id) {
                                name.to_string()
                            } else {
                                // Otherwise, assign new name
                                let mut i = 0;
                                let mut name = def.name.as_str().into();
                                while subtypes.contains_key(&name) {
                                    i += 1;
                                    name = format!("{}_{i}", def.name.as_str());
                                }

                                subtypes.insert(name.clone(), id);

                                // Recursively resolve fields to populate subtypes
                                for field_id in def.fields {
                                    let field_ty = &self.hir.variable(*field_id).ty;
                                    self.resolve_type(field_ty, subtypes)?;
                                }
                                name
                            };

                        Some(name)
                    }
                    // Rest of `ItemId` are not supported by EIP-712
                    _ => None,
                }
            }
            // EIP-712 doesn't support functions, mappings, nor errors
            TypeKind::Mapping(_) | TypeKind::Function { .. } | TypeKind::Err(_) => None,
        }
    }
}

fn parse_array_size<'hir>(expr: &Expr<'hir>) -> Option<String> {
    if let ExprKind::Lit(lit) = &expr.kind {
        if let LitKind::Number(int) = &lit.kind {
            return Some(int.to_string());
        }
    }

    None
}
