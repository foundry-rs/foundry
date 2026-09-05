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
        let gcx = ctx.gcx();
        let source_map = gcx.sess.source_map();
        let input_sources: HashMap<_, _> = gcx
            .hir
            .sources_enumerated()
            .filter_map(|(sid, src)| {
                let FileName::Real(path) = &src.file.name else { return None };
                Some((sid, sources.iter().position(|s| &s.path == path)?))
            })
            .collect();

        // The spellings each contract uses across all of its variables.
        let mut contracts = HashMap::<hir::ContractId, Vec<&str>>::new();
        for variable in gcx.hir.variables() {
            if let Some(contract_id) = variable.contract
                && input_sources.contains_key(&variable.source)
            {
                int_spellings(source_map, &variable.ty, contracts.entry(contract_id).or_default());
            }
        }

        // HIR variable order is stable, so diagnostics remain deterministic across runs.
        for variable in gcx.hir.variables() {
            let Some(contract_id) = variable.contract else { continue };
            let Some(&source_idx) = input_sources.get(&variable.source) else { continue };
            let Some(contract_names) = contracts.get(&contract_id) else { continue };
            let mut names = Vec::new();
            int_spellings(source_map, &variable.ty, &mut names);
            if (names.contains(&"uint") && contract_names.contains(&"uint256"))
                || (names.contains(&"int") && contract_names.contains(&"int256"))
            {
                ctx.emit(&sources[source_idx], &INCONSISTENT_TYPE_NAMES, variable.span);
            }
        }
    }
}

/// Collects the 256-bit integer spellings (`int`, `int256`, `uint`, `uint256`) of a declared type.
///
/// Solar normalizes `int`/`uint` to their 256-bit forms in HIR, so the exact source span of an
/// already typed 256-bit integer node recovers which spelling the declaration used; a missing or
/// unexpected snippet is ignored conservatively. Function-type parameters and returns are
/// separate HIR variables, so function types are not recursed here, matching the upstream
/// detector's treatment of arrays and mappings.
fn int_spellings(source_map: &SourceMap, ty: &hir::Type<'_>, out: &mut Vec<&'static str>) {
    match &ty.kind {
        TypeKind::Elementary(ElementaryType::Int(size) | ElementaryType::UInt(size))
            if size.bits() == 256 =>
        {
            if let Ok(snippet) = source_map.span_to_snippet(ty.span)
                && let Some(spelling) =
                    ["int", "int256", "uint", "uint256"].into_iter().find(|s| *s == snippet)
            {
                out.push(spelling);
            }
        }
        TypeKind::Array(array) => int_spellings(source_map, &array.element, out),
        TypeKind::Mapping(mapping) => {
            int_spellings(source_map, &mapping.key, out);
            int_spellings(source_map, &mapping.value, out);
        }
        _ => {}
    }
}
