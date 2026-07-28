use crate::{
    linter::{Lint, ProjectLintEmitter, ProjectLintPass, ProjectSource},
    sol::{Severity, SolLint, low::InconsistentTypeNames},
};
use solar::{
    ast::ElementaryType,
    interface::{SourceMap, source_map::FileName},
    sema::hir::{self, TypeKind},
};
use std::collections::HashMap;

declare_forge_lint!(
    INCONSISTENT_TYPE_NAMES,
    Severity::Low,
    "inconsistent-type-names",
    "use explicit `uint256` and `int256` type names consistently within a contract"
);

impl<'ast> ProjectLintPass<'ast> for InconsistentTypeNames {
    fn check_project(&mut self, ctx: &ProjectLintEmitter<'_, '_>, sources: &[ProjectSource<'ast>]) {
        if !ctx.is_lint_enabled(INCONSISTENT_TYPE_NAMES.id()) {
            return;
        }

        let hir = &ctx.gcx().hir;
        let source_map = ctx.gcx().sess.source_map();
        let input_sources: HashMap<_, _> = hir
            .sources_enumerated()
            .filter_map(|(source_id, source)| {
                let FileName::Real(path) = &source.file.name else { return None };
                let source_idx = sources.iter().position(|source| &source.path == path)?;
                Some((source_id, source_idx))
            })
            .collect();

        let mut contracts = HashMap::<hir::ContractId, TypeNames>::new();
        for variable in hir.variables() {
            let Some(contract_id) = variable.contract else { continue };
            if !input_sources.contains_key(&variable.source) {
                continue;
            }

            contracts.entry(contract_id).or_default().merge(type_names(source_map, &variable.ty));
        }

        // HIR variable order is stable, so diagnostics remain deterministic across runs.
        for variable in hir.variables() {
            let Some(contract_id) = variable.contract else { continue };
            let Some(&source_idx) = input_sources.get(&variable.source) else { continue };
            let Some(contract_names) = contracts.get(&contract_id) else { continue };
            let variable_names = type_names(source_map, &variable.ty);

            if (variable_names.has_uint && contract_names.has_uint256)
                || (variable_names.has_int && contract_names.has_int256)
            {
                ctx.emit(&sources[source_idx], &INCONSISTENT_TYPE_NAMES, variable.span);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct TypeNames {
    has_int: bool,
    has_int256: bool,
    has_uint: bool,
    has_uint256: bool,
}

impl TypeNames {
    const fn merge(&mut self, other: Self) {
        self.has_int |= other.has_int;
        self.has_int256 |= other.has_int256;
        self.has_uint |= other.has_uint;
        self.has_uint256 |= other.has_uint256;
    }
}

/// Collects integer spellings from a single resolved variable declaration.
///
/// Function-type parameters and returns are separate HIR variables, so function types are not
/// recursed here. This keeps each shorthand declaration to one diagnostic while matching the
/// upstream detector's treatment of arrays and mappings.
fn type_names(source_map: &SourceMap, ty: &hir::Type<'_>) -> TypeNames {
    let mut names = TypeNames::default();
    match &ty.kind {
        // Solar normalizes `int`/`uint` to their 256-bit forms in HIR. Inspect only the exact
        // source span of an already typed 256-bit integer node to recover which spelling the
        // declaration used. A missing or unexpected snippet is ignored conservatively.
        TypeKind::Elementary(ElementaryType::Int(size)) if size.bits() == 256 => {
            match source_map.span_to_snippet(ty.span).as_deref() {
                Ok("int") => names.has_int = true,
                Ok("int256") => names.has_int256 = true,
                _ => {}
            }
        }
        TypeKind::Elementary(ElementaryType::UInt(size)) if size.bits() == 256 => {
            match source_map.span_to_snippet(ty.span).as_deref() {
                Ok("uint") => names.has_uint = true,
                Ok("uint256") => names.has_uint256 = true,
                _ => {}
            }
        }
        TypeKind::Array(array) => names.merge(type_names(source_map, &array.element)),
        TypeKind::Mapping(mapping) => {
            names.merge(type_names(source_map, &mapping.key));
            names.merge(type_names(source_map, &mapping.value));
        }
        TypeKind::Function(_) | TypeKind::Custom(_) | TypeKind::Err(_) => {}
        TypeKind::Elementary(_) => {}
    }
    names
}
