use super::IncorrectUsingFor;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::DataLocation,
    sema::{
        Gcx,
        hir::{self, UsingDirective, UsingEntryKind},
    },
};

declare_forge_lint!(
    INCORRECT_USING_FOR,
    Severity::Info,
    "incorrect-using-for",
    "`using ... for` names a library with no function applicable to the type, so the directive attaches nothing"
);

impl<'gcx> LateLintPass<'gcx> for IncorrectUsingFor {
    fn check_nested_source(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, id: hir::SourceId) {
        for directive in gcx.hir.source(id).usings {
            check_directive(ctx, gcx, directive);
        }
    }

    fn check_nested_contract(&mut self, ctx: &LintContext, gcx: Gcx<'gcx>, id: hir::ContractId) {
        for directive in gcx.hir.contract(id).usings {
            check_directive(ctx, gcx, directive);
        }
    }
}

/// Judges one `using ... for` directive: a library entry that contributes no member to the
/// target type attaches nothing, which means no function of the library accepts the type as
/// its bound first parameter.
fn check_directive<'gcx>(ctx: &LintContext, gcx: Gcx<'gcx>, directive: &'gcx UsingDirective<'gcx>) {
    // `using L for *` attaches every function of the library: nothing to validate.
    let Some(hir_ty) = &directive.ty else { return };
    // `members_of` expects reference types wrapped in their data location. Storage converts
    // implicitly to memory but not to calldata, so a library function whose bound first
    // parameter is `calldata` only shows up under the calldata form: each location is probed
    // and the directive only flags when none of them attaches.
    let base_ty = gcx.type_of_hir_ty(hir_ty);
    let tys = [DataLocation::Storage, DataLocation::Memory, DataLocation::Calldata]
        .map(|loc| base_ty.with_loc_if_ref(gcx, loc));
    for entry in directive.entries {
        // The braced form `using {f} for T` is already type-checked: the compiler rejects a
        // function that cannot attach to `T`.
        let UsingEntryKind::Library(library_id) = entry.kind else { continue };
        // A member counts when it is an attached, non-private function declared in the library:
        // the library form skips private functions, so a private member attached by a braced
        // directive in scope does not make this entry live.
        let attaches = tys
            .iter()
            .flat_map(|ty| gcx.members_of(*ty, directive.source, directive.contract))
            .filter(|member| member.attached)
            .filter_map(|member| member.ty.function_id())
            .any(|function_id| {
                let function = gcx.hir.function(function_id);
                function.contract == Some(library_id)
                    && function.visibility != hir::Visibility::Private
            });
        if !attaches {
            ctx.emit(&INCORRECT_USING_FOR, entry.span);
        }
    }
}
