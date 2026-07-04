use super::IncorrectUsingFor;
use crate::{
    linter::{LateLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::DataLocation,
    sema::{
        Gcx,
        hir::{self, Hir, UsingDirective, UsingEntryKind},
        ty::TyKind,
    },
};

declare_forge_lint!(
    INCORRECT_USING_FOR,
    Severity::Info,
    "incorrect-using-for",
    "`using ... for` names a library with no function applicable to the type, so the directive attaches nothing"
);

impl<'hir> LateLintPass<'hir> for IncorrectUsingFor {
    fn check_nested_source(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        id: hir::SourceId,
    ) {
        // The file-level directives.
        for directive in hir.source(id).usings {
            self.check_directive(ctx, gcx, hir, directive);
        }
    }

    fn check_nested_contract(
        &mut self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        id: hir::ContractId,
    ) {
        // The contract-level directives.
        for directive in hir.contract(id).usings {
            self.check_directive(ctx, gcx, hir, directive);
        }
    }
}

impl IncorrectUsingFor {
    /// Judges one `using ... for` directive: a library entry that contributes no member to the
    /// target type attaches nothing, which means no function of the library accepts the type
    /// as its bound first parameter.
    fn check_directive<'hir>(
        &self,
        ctx: &LintContext,
        gcx: Gcx<'hir>,
        hir: &'hir Hir<'hir>,
        directive: &'hir UsingDirective<'hir>,
    ) {
        // `using L for *` attaches every function of the library: nothing to validate.
        let Some(hir_ty) = &directive.ty else { return };
        // `members_of` expects reference types wrapped in their data location. Storage
        // converts implicitly to memory, so the storage form covers library functions
        // taking either location.
        let ty = gcx.type_of_hir_ty(hir_ty).with_loc_if_ref(gcx, DataLocation::Storage);
        for entry in directive.entries {
            // The braced form `using {f} for T` is already type-checked: the compiler rejects
            // a function that cannot attach to `T`.
            let UsingEntryKind::Library(library_id) = entry.kind else { continue };
            // The directive is useful when at least one member the type gains in this scope
            // comes from the named library. Whether the bound value converts to the first
            // parameter is the type checker's business: `members_of` already reflects it.
            let mut attaches = false;
            for member in gcx.members_of(ty, directive.source, directive.contract) {
                // A member counts when it is an attached function declared in the library.
                if member.attached
                    && let TyKind::Fn(function_ty) = member.ty.kind
                    && let Some(function_id) = function_ty.function_id
                    && hir.function(function_id).contract == Some(library_id)
                {
                    attaches = true;
                }
            }
            // No function of the library accepts the type: the directive is a no-op.
            if !attaches {
                ctx.emit(&INCORRECT_USING_FOR, entry.span);
            }
        }
    }
}
