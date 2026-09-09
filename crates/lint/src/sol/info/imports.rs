use super::Imports;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};
use solar::{
    ast::{self as ast, SourceUnit, Symbol, visit::Visit},
    data_structures::{Never, map::FxHashSet},
    interface::SourceMap,
};
use std::ops::ControlFlow;

declare_forge_lint!(
    UNUSED_IMPORT,
    Severity::Info,
    "unused-import",
    "unused imports should be removed"
);

declare_forge_lint!(
    UNALIASED_PLAIN_IMPORT,
    Severity::Info,
    "unaliased-plain-import",
    "use named imports '{A, B}' or alias 'import \"..\" as X'"
);

impl<'ast> EarlyLintPass<'ast> for Imports {
    fn check_import_directive(
        &mut self,
        ctx: &LintContext,
        import: &'ast ast::ImportDirective<'ast>,
    ) {
        // Non-aliased plain imports like `import "File.sol";`.
        if let ast::ImportItems::Plain(_) = &import.items
            && import.source_alias().is_none()
        {
            ctx.emit(&UNALIASED_PLAIN_IMPORT, import.path.span);
        }
    }

    fn check_full_source_unit(&mut self, ctx: &LintContext<'ast, '_>, ast: &'ast SourceUnit<'ast>) {
        // Disabled lints are filtered inside `ctx.emit()`, but the full traversal is expensive.
        if !ctx.is_lint_enabled(UNUSED_IMPORT.id) {
            return;
        }
        let mut checker =
            UsedSymbols { source_map: ctx.session().source_map(), used: FxHashSet::default() };
        let _ = checker.visit_source_unit(ast);
        let used = checker.used;

        for item in ast.items.iter() {
            let ast::ItemKind::Import(import) = &item.kind else { continue };
            match &import.items {
                ast::ImportItems::Aliases(symbols) => {
                    for &(orig, alias) in symbols.iter() {
                        let name = alias.unwrap_or(orig);
                        if !used.contains(&name.name) {
                            ctx.emit(&UNUSED_IMPORT, orig.span.to(name.span));
                        }
                    }
                }
                ast::ImportItems::Plain(_) | ast::ImportItems::Glob(_) => {
                    if let Some(alias) = import.source_alias()
                        && !used.contains(&alias.name)
                    {
                        ctx.emit(&UNUSED_IMPORT, item.span);
                    }
                }
            }
        }
    }
}

/// Collects every symbol a source unit refers to outside its import directives.
struct UsedSymbols<'ast> {
    source_map: &'ast SourceMap,
    used: FxHashSet<Symbol>,
}

impl<'ast> Visit<'ast> for UsedSymbols<'ast> {
    type BreakValue = Never;

    fn visit_item(&mut self, item: &'ast ast::Item<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::ItemKind::Import(_) = &item.kind {
            return ControlFlow::Continue(());
        }
        self.walk_item(item)
    }

    fn visit_using_directive(
        &mut self,
        using: &'ast ast::UsingDirective<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        match &using.list {
            ast::UsingList::Single(path) => {
                self.used.insert(path.first().name);
            }
            ast::UsingList::Multiple(items) => {
                self.used.extend(items.iter().map(|(path, _)| path.first().name));
            }
        }
        self.walk_using_directive(using)
    }

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::ExprKind::Ident(id) = expr.kind {
            self.used.insert(id.name);
        }
        self.walk_expr(expr)
    }

    fn visit_path(&mut self, path: &'ast ast::PathSlice) -> ControlFlow<Self::BreakValue> {
        self.used.extend(path.segments().iter().map(|id| id.name));
        self.walk_path(path)
    }

    fn visit_ty(&mut self, ty: &'ast ast::Type<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::TypeKind::Custom(path) = &ty.kind {
            self.used.insert(path.first().name);
        }
        self.walk_ty(ty)
    }

    fn visit_doc_comment(&mut self, cmnt: &'ast ast::DocComment) -> ControlFlow<Self::BreakValue> {
        if let Ok(snip) = self.source_map.span_to_snippet(cmnt.span) {
            for line in snip.lines() {
                if let Some((_, relevant)) = line.split_once("@inheritdoc") {
                    self.used.insert(Symbol::intern(relevant.trim()));
                }
            }
        }
        ControlFlow::Continue(())
    }
}
