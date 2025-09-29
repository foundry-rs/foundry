use alloy_primitives::{B256, keccak256};
use clap::{Parser, ValueHint};
use eyre::Result;
use foundry_cli::{opts::BuildOpts, utils::LoadConfig};
use foundry_common::{compile::ProjectCompiler, shell};
use serde::Serialize;
use solar::sema::{
    Gcx, Hir,
    hir::StructId,
    ty::{Ty, TyKind},
};
use std::{
    collections::BTreeMap,
    fmt::{Display, Formatter, Result as FmtResult, Write},
    ops::ControlFlow,
    path::{Path, PathBuf},
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

#[derive(Debug, Serialize)]
struct Eip712Output {
    path: String,
    #[serde(rename = "type")]
    ty: String,
    hash: B256,
}

impl Display for Eip712Output {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        writeln!(f, "{}:", self.path)?;
        writeln!(f, " - type: {}", self.ty)?;
        writeln!(f, " - hash: {}", self.hash)
    }
}

impl Eip712Args {
    pub fn run(self) -> Result<()> {
        let config = self.build.load_config()?;
        let project = config.solar_project()?;
        let mut output = ProjectCompiler::new().files([self.target_path]).compile(&project)?;
        let compiler = output.parser_mut().solc_mut().compiler_mut();
        compiler.enter_mut(|compiler| -> Result<()> {
            let Ok(ControlFlow::Continue(())) = compiler.lower_asts() else { return Ok(()) };
            let gcx = compiler.gcx();
            let resolver = Resolver::new(gcx);

            let outputs = resolver
                .struct_ids()
                .filter_map(|id| {
                    let resolved = resolver.resolve_struct_eip712(id)?;
                    Some(Eip712Output {
                        path: resolver.get_struct_path(id),
                        hash: keccak256(resolved.as_bytes()),
                        ty: resolved,
                    })
                })
                .collect::<Vec<_>>();

            if shell::is_json() {
                sh_println!("{json}", json = serde_json::to_string_pretty(&outputs)?)?;
            } else {
                for output in &outputs {
                    sh_println!("{output}")?;
                }
            }

            Ok(())
        })?;

        // `compiler.sess()` inside of `ProjectCompileOutput` is built with `with_buffer_emitter`.
        let diags = compiler.sess().dcx.emitted_diagnostics().unwrap();
        if compiler.sess().dcx.has_errors().is_err() {
            eyre::bail!("{diags}");
        } else {
            let _ = sh_print!("{diags}");
        }

        Ok(())
    }
}

/// Generates the EIP-712 `encodeType` string for a given struct.
///
/// Requires a reference to the source HIR.
pub struct Resolver<'gcx> {
    gcx: Gcx<'gcx>,
}

impl<'gcx> Resolver<'gcx> {
    /// Constructs a new [`Resolver`] for the supplied [`Hir`] instance.
    pub fn new(gcx: Gcx<'gcx>) -> Self {
        Self { gcx }
    }

    #[inline]
    fn hir(&self) -> &'gcx Hir<'gcx> {
        &self.gcx.hir
    }

    /// Returns the [`StructId`]s of every user-defined struct in source order.
    pub fn struct_ids(&self) -> impl Iterator<Item = StructId> {
        self.hir().strukt_ids()
    }

    /// Returns the path for a struct, with the format: `file.sol > MyContract > MyStruct`
    pub fn get_struct_path(&self, id: StructId) -> String {
        let strukt = self.hir().strukt(id).name.as_str();
        match self.hir().strukt(id).contract {
            Some(cid) => {
                let full_name = self.gcx.contract_fully_qualified_name(cid).to_string();
                let relevant = Path::new(&full_name)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&full_name);

                if let Some((file, contract)) = relevant.rsplit_once(':') {
                    format!("{file} > {contract} > {strukt}")
                } else {
                    format!("{relevant} > {strukt}")
                }
            }
            None => strukt.to_string(),
        }
    }

    /// Converts a given struct into its EIP-712 `encodeType` representation.
    ///
    /// Returns `None` if the struct, or any of its fields, contains constructs
    /// not supported by EIP-712 (mappings, function types, errors, etc).
    pub fn resolve_struct_eip712(&self, id: StructId) -> Option<String> {
        let mut subtypes = BTreeMap::new();
        subtypes.insert(self.hir().strukt(id).name.as_str().into(), id);
        self.resolve_eip712_inner(id, &mut subtypes, true, None)
    }

    fn resolve_eip712_inner(
        &self,
        id: StructId,
        subtypes: &mut BTreeMap<String, StructId>,
        append_subtypes: bool,
        rename: Option<&str>,
    ) -> Option<String> {
        let def = self.hir().strukt(id);
        let mut result = format!("{}(", rename.unwrap_or(def.name.as_str()));

        for (idx, field_id) in def.fields.iter().enumerate() {
            let field = self.hir().variable(*field_id);
            let ty = self.resolve_type(self.gcx.type_of_hir_ty(&field.ty), subtypes)?;

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
                    continue;
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
        ty: Ty<'gcx>,
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
                let def = self.hir().strukt(id);
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
                        for &field_id in def.fields {
                            let field_ty = self.gcx.type_of_item(field_id.into());
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
