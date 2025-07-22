//! Semantic analysis helpers for extracting type information and other useful metadata from the
//! HIR.

use eyre::{Result, eyre};
use solar_sema::{
    GcxWrapper, Hir, hir,
    ty::{Ty, TyKind},
};
use std::{collections::HashMap, ops::Deref, sync::Arc};

#[derive(Debug, Clone)]
pub struct StructDefinitions(Arc<HashMap<String, Vec<(String, String)>>>);

impl StructDefinitions {
    pub fn new(map: HashMap<String, Vec<(String, String)>>) -> Self {
        StructDefinitions(Arc::new(map))
    }
}

impl Default for StructDefinitions {
    fn default() -> Self {
        StructDefinitions(Arc::new(HashMap::new()))
    }
}

impl Deref for StructDefinitions {
    type Target = HashMap<String, Vec<(String, String)>>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Arc<HashMap<String, Vec<(String, String)>>>> for StructDefinitions {
    fn as_ref(&self) -> &Arc<HashMap<String, Vec<(String, String)>>> {
        &self.0
    }
}

/// Generates a map of all struct definitions from the HIR using the resolved `Ty` system.
pub struct SemanticAnalysisProcessor<'hir> {
    gcx: GcxWrapper<'hir>,
    struct_defs: HashMap<String, Vec<(String, String)>>,
}

impl<'hir> SemanticAnalysisProcessor<'hir> {
    /// Constructs a new generator.
    pub fn new(gcx: GcxWrapper<'hir>) -> Self {
        Self { gcx, struct_defs: HashMap::new() }
    }

    /// Processes the HIR to generate all the struct definitions.
    pub fn process(mut self) -> Result<Self> {
        for id in self.hir().strukt_ids() {
            self.resolve_struct_definition(id)?;
        }

        Ok(self)
    }

    pub fn struct_defs(self) -> StructDefinitions {
        StructDefinitions(Arc::new(self.struct_defs))
    }

    #[inline]
    fn hir(&self) -> &'hir Hir<'hir> {
        &self.gcx.get().hir
    }

    /// The recursive core of the generator. Resolves a single struct and adds it to the cache.
    fn resolve_struct_definition(&mut self, id: hir::StructId) -> Result<()> {
        let qualified_name = self.get_fully_qualified_name(id);
        if self.struct_defs.contains_key(&qualified_name) {
            return Ok(());
        }

        let gcx = self.gcx.get();
        let hir = &gcx.hir;
        let strukt = hir.strukt(id);
        let mut fields = Vec::with_capacity(strukt.fields.len());

        for &field_id in strukt.fields {
            let var = hir.variable(field_id);
            let name = var.name.ok_or_else(|| eyre!("Struct field is missing a name"))?.to_string();
            if let Some(ty_str) = self.ty_to_string(gcx.type_of_hir_ty(&var.ty)) {
                fields.push((name, ty_str));
            }
        }

        if !fields.is_empty() {
            self.struct_defs.insert(qualified_name, fields);
        }

        Ok(())
    }

    /// Converts a resolved `Ty` into its canonical string representation.
    fn ty_to_string(&mut self, ty: Ty<'hir>) -> Option<String> {
        let ty = ty.peel_refs();
        let res = match ty.kind {
            TyKind::Elementary(e) => e.to_string(),
            TyKind::Array(ty, size) => {
                let inner_type = self.ty_to_string(ty)?;
                format!("{}[{}]", inner_type, size)
            }
            TyKind::DynArray(ty) => {
                let inner_type = self.ty_to_string(ty)?;
                format!("{}[]", inner_type)
            }
            TyKind::Struct(id) => {
                // Ensure the nested struct is resolved before proceeding.
                self.resolve_struct_definition(id).ok()?;
                self.get_fully_qualified_name(id)
            }
            TyKind::Udvt(ty, _) => self.ty_to_string(ty)?,
            // For now, map enums to `uint8`
            TyKind::Enum(_) => "uint8".to_string(),
            // For now, map contracts to `address`
            TyKind::Contract(_) => "address".to_string(),
            // Explicitly disallow unsupported types
            _ => return None,
        };

        Some(res)
    }

    /// Helper to get the fully qualified name `Contract.Struct`.
    fn get_fully_qualified_name(&self, id: hir::StructId) -> String {
        let hir = self.hir();
        let strukt = hir.strukt(id);
        if let Some(contract_id) = strukt.contract {
            let contract_name = hir.contract(contract_id).name.as_str();
            format!("{}.{}", contract_name, strukt.name.as_str())
        } else {
            strukt.name.as_str().to_string()
        }
    }
}
