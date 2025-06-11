use alloy_primitives::keccak256;
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::opts::{solar_pcx_from_build_opts, BuildOpts};
use solar_parse::interface::Session;
use solar_sema::{
    hir::StructId,
    thread_local::ThreadLocal,
    ty::{Ty, TyKind},
    GcxWrapper, Hir,
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

        let result = sess.enter(|| -> Result<()> {
            // Set up the parsing context with the project paths and sources.
            let parsing_context =
                solar_pcx_from_build_opts(&sess, self.build, Some(vec![self.target_path]))?;

            // Parse and resolve
            let hir_arena = ThreadLocal::new();
            if let Ok(Some(gcx)) = parsing_context.parse_and_lower(&hir_arena) {
                let resolver = Resolver::new(gcx);
                for id in &resolver.struct_ids() {
                    if let Some(resolved) = resolver.resolve_struct_eip712(*id) {
                        _ = sh_println!(
                            "{label}:\n - type: {resolved}\n - hash: {hash}\n",
                            label = resolver.get_struct_label(*id),
                            hash = keccak256(resolved.as_bytes())
                        );
                    }
                }
            }

            Ok(())
        });

        eyre::ensure!(result.is_ok() && sess.dcx.has_errors().is_ok(), "failed parsing");

        Ok(())
    }
}

/// Generates the EIP-712 `encodeType` string for a given struct.
///
/// Requires a reference to the source HIR.
pub struct Resolver<'hir> {
    hir: &'hir Hir<'hir>,
    gcx: GcxWrapper<'hir>,
}

impl<'hir> Resolver<'hir> {
    /// Constructs a new [`Resolver`] for the supplied [`Hir`] instance.
    pub fn new(gcx: GcxWrapper<'hir>) -> Self {
        Self { hir: &gcx.get().hir, gcx }
    }

    /// Returns the [`StructId`]s of every user-defined struct in source order.
    pub fn struct_ids(&self) -> Vec<StructId> {
        self.hir.strukt_ids().collect()
    }

    /// Returns the label for a struct, with the format: `file.sol > MyContract > MyStruct`
    pub fn get_struct_label(&self, id: StructId) -> String {
        let strukt = self.hir.strukt(id).name.as_str();
        self.hir.strukt(id).contract.map_or(String::new(), |cid| {
            let full_name = self.gcx.get().contract_fully_qualified_name(cid).to_string();
            let relevant = full_name.rsplit_once('/').map_or(&*full_name, |(_, part)| part);

            if let Some((file, contract)) = relevant.rsplit_once(':') {
                format!("{file} > {contract} > {strukt}")
            } else {
                format!("{relevant} > {strukt}")
            }
        })
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
            let ty = self.resolve_type(self.gcx.get().type_of_hir_ty(&field.ty), subtypes)?;

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
        ty: Ty<'hir>,
        subtypes: &mut BTreeMap<String, StructId>,
    ) -> Option<String> {
        let ty = ty.peel_refs();
        match ty.kind {
            TyKind::Elementary(elem_ty) => Some(elem_ty.to_abi_str().to_string()),
            TyKind::Array(element_ty, size) => {
                let inner_type = self.resolve_type(element_ty, subtypes)?;
                let size = size.to_string();
                Some(format!("{inner_type}[{size}]"))
            }
            TyKind::DynArray(element_ty) => {
                let inner_type = self.resolve_type(element_ty, subtypes)?;
                Some(format!("{inner_type}[]"))
            }
            TyKind::Udvt(ty, _) => self.resolve_type(ty, subtypes),
            TyKind::Struct(id) => {
                let def = self.hir.strukt(id);
                let name = match subtypes.iter().find(|(_, cached_id)| id == **cached_id) {
                    Some((name, _)) => name.to_string(),
                    None => {
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
                            let field_ty =
                                self.gcx.get().type_of_hir_ty(&self.hir.variable(*field_id).ty);
                            self.resolve_type(field_ty, subtypes)?;
                        }
                        name
                    }
                };

                Some(name)
            }
            // For now, map enums to `uint8`
            TyKind::Enum(_) => Some("uint8".to_string()),
            // For now, map contracts to `address`
            TyKind::Contract(_) => Some("address".to_string()),
            // EIP-712 doesn't support tuples (should use structs), functions, mappings, nor errors
            _ => None,
        }
    }
}
