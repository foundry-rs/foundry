//! Cheatcode information, extracted from the syntactic and semantic analysis of the sources.

use foundry_common::fmt::{StructDefinitions, TypeDefMap};
use solar::sema::{self, Compiler, Gcx, hir};
use std::sync::{Arc, OnceLock};
use thiserror::Error;

/// Represents a failure in one of the lazy analysis steps.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AnalysisError {
    /// Indicates that the resolution of struct definitions failed.
    #[error("unable to resolve struct definitions")]
    StructDefinitionsResolutionFailed,
}

/// Provides cached, on-demand syntactic and semantic analysis of a completed `Compiler` instance.
///
/// This struct acts as a facade over the `Compiler`, offering lazy-loaded analysis for tools like
/// cheatcode inspectors. It assumes the compiler has already completed parsing and lowering.
///
/// # Adding with new analyses types
///
/// To add support for a new type of cached analysis, follow this pattern:
///
/// 1. Add a new `pub OnceCell<Result<T, AnalysisError>>` field to `CheatcodeAnalysis`, where `T` is
///    the type of the data that you are adding support for.
///
/// 2. Implement a getter method for the new field. Inside the getter, use
///    `self.field.get_or_init()` to compute and cache the value on the first call.
///
/// 3. Inside the closure passed to `get_or_init()`, create a dedicated visitor to traverse the HIR
///    using `self.compiler.enter()` and collect the required data.
///
/// This ensures all analyses remain lazy, efficient, and consistent with the existing design.
#[derive(Clone)]
pub struct CheatcodeAnalysis {
    /// A shared, thread-safe reference to solar's `Compiler` instance.
    pub compiler: Arc<Compiler>,

    /// Cached struct definitions in the sources.
    /// Used to keep field order when parsing JSON values.
    struct_defs: OnceLock<Result<StructDefinitions, AnalysisError>>,
}

impl std::fmt::Debug for CheatcodeAnalysis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheatcodeAnalysis")
            .field("compiler", &"<compiler>")
            .field("struct_defs", &self.struct_defs)
            .finish()
    }
}

impl CheatcodeAnalysis {
    pub fn new(compiler: Arc<solar::sema::Compiler>) -> Self {
        Self { compiler, struct_defs: OnceLock::new() }
    }

    /// Lazily initializes and returns the struct definitions.
    pub fn struct_defs(&self) -> Result<&StructDefinitions, &AnalysisError> {
        self.struct_defs
            .get_or_init(|| {
                self.compiler.enter(|compiler| {
                    let gcx = compiler.gcx();

                    StructDefinitionResolver::new(gcx).process()
                })
            })
            .as_ref()
    }
}

// -- STRUCT DEFINITIONS -------------------------------------------------------

/// Generates a map of all struct definitions from the HIR using the resolved `Ty` system.
struct StructDefinitionResolver<'gcx> {
    gcx: Gcx<'gcx>,
    struct_defs: TypeDefMap,
}

impl<'gcx> StructDefinitionResolver<'gcx> {
    /// Constructs a new generator.
    pub fn new(gcx: Gcx<'gcx>) -> Self {
        Self { gcx, struct_defs: TypeDefMap::new() }
    }

    /// Processes the HIR to generate all the struct definitions.
    pub fn process(mut self) -> Result<StructDefinitions, AnalysisError> {
        for id in self.hir().strukt_ids() {
            self.resolve_struct_definition(id)?;
        }
        Ok(self.struct_defs.into())
    }

    #[inline]
    fn hir(&self) -> &'gcx hir::Hir<'gcx> {
        &self.gcx.hir
    }

    /// The recursive core of the generator. Resolves a single struct and adds it to the cache.
    fn resolve_struct_definition(&mut self, id: hir::StructId) -> Result<(), AnalysisError> {
        let qualified_name = self.get_fully_qualified_name(id);
        if self.struct_defs.contains_key(&qualified_name) {
            return Ok(());
        }

        let hir = self.hir();
        let strukt = hir.strukt(id);
        let mut fields = Vec::with_capacity(strukt.fields.len());

        for &field_id in strukt.fields {
            let var = hir.variable(field_id);
            let name =
                var.name.ok_or(AnalysisError::StructDefinitionsResolutionFailed)?.to_string();
            if let Some(ty_str) = self.ty_to_string(self.gcx.type_of_hir_ty(&var.ty)) {
                fields.push((name, ty_str));
            }
        }

        // Only insert if there are fields, to avoid adding empty entries
        if !fields.is_empty() {
            self.struct_defs.insert(qualified_name, fields);
        }

        Ok(())
    }

    /// Converts a resolved `Ty` into its canonical string representation.
    fn ty_to_string(&mut self, ty: sema::Ty<'gcx>) -> Option<String> {
        let ty = ty.peel_refs();
        let res = match ty.kind {
            sema::ty::TyKind::Elementary(e) => e.to_string(),
            sema::ty::TyKind::Array(ty, size) => {
                let inner_type = self.ty_to_string(ty)?;
                format!("{inner_type}[{size}]")
            }
            sema::ty::TyKind::DynArray(ty) => {
                let inner_type = self.ty_to_string(ty)?;
                format!("{inner_type}[]")
            }
            sema::ty::TyKind::Struct(id) => {
                // Ensure the nested struct is resolved before proceeding.
                self.resolve_struct_definition(id).ok()?;
                self.get_fully_qualified_name(id)
            }
            sema::ty::TyKind::Udvt(ty, _) => self.ty_to_string(ty)?,
            // For now, map enums to `uint8`
            sema::ty::TyKind::Enum(_) => "uint8".to_string(),
            // For now, map contracts to `address`
            sema::ty::TyKind::Contract(_) => "address".to_string(),
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
            format!("{}.{}", hir.contract(contract_id).name.as_str(), strukt.name.as_str())
        } else {
            strukt.name.as_str().into()
        }
    }
}
