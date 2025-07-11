use solar_ast::{self as ast, SourceUnit, Span, Symbol, visit::Visit};
use solar_data_structures::map::FxIndexSet;
use std::ops::ControlFlow;

use super::Imports;
use crate::{
    linter::{EarlyLintPass, LintContext},
    sol::{Severity, SolLint},
};

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
        ctx: &LintContext<'_>,
        import: &'ast ast::ImportDirective<'ast>,
    ) {
        // Non-aliased plain imports like `import "File.sol";`.
        if let ast::ImportItems::Plain(_) = &import.items
            && import.source_alias().is_none()
        {
            ctx.emit(&UNALIASED_PLAIN_IMPORT, import.path.span);
        }
    }

    fn check_full_source_unit(&mut self, ctx: &LintContext<'_>, ast: &'ast SourceUnit<'ast>) {
        let mut checker = UnusedChecker::new();
        let _ = checker.visit_source_unit(ast);
        checker.check_unused_imports(ast, ctx);
        checker.clear();
    }
}

/// Visitor that collects all used symbols in a source unit.
struct UnusedChecker {
    used_symbols: FxIndexSet<Symbol>,
}

impl UnusedChecker {
    fn new() -> Self {
        Self { used_symbols: Default::default() }
    }

    fn clear(&mut self) {
        self.used_symbols.clear();
    }

    /// Mark a symbol as used in a source.
    fn mark_symbol_used(&mut self, symbol: Symbol) {
        self.used_symbols.insert(symbol);
    }

    /// Check for unused imports and emit warnings.
    fn check_unused_imports(&self, ast: &SourceUnit<'_>, ctx: &LintContext<'_>) {
        for item in ast.items.iter() {
            let span = item.span;
            let ast::ItemKind::Import(import) = &item.kind else { continue };
            match &import.items {
                ast::ImportItems::Plain(_) | ast::ImportItems::Glob(_) => {
                    if let Some(alias) = import.source_alias()
                        && !self.used_symbols.contains(&alias.name)
                    {
                        self.unused_import(ctx, span);
                    }
                }
                ast::ImportItems::Aliases(symbols) => {
                    for &(orig, alias) in symbols.iter() {
                        let name = alias.unwrap_or(orig);
                        if !self.used_symbols.contains(&name.name) {
                            self.unused_import(ctx, orig.span.to(name.span));
                        }
                    }
                }
            }
        }
    }

    fn unused_import(&self, ctx: &LintContext<'_>, span: Span) {
        ctx.emit(&UNUSED_IMPORT, span);
    }
}

impl<'ast> Visit<'ast> for UnusedChecker {
    type BreakValue = solar_data_structures::Never;

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
                self.mark_symbol_used(path.first().name);
            }
            ast::UsingList::Multiple(items) => {
                for (path, _) in items.iter() {
                    self.mark_symbol_used(path.first().name);
                }
            }
        }

        self.walk_using_directive(using)
    }

    fn visit_modifier(
        &mut self,
        modifier: &'ast ast::Modifier<'ast>,
    ) -> ControlFlow<Self::BreakValue> {
        self.mark_symbol_used(modifier.name.first().name);

        self.walk_modifier(modifier)
    }

    fn visit_expr(&mut self, expr: &'ast ast::Expr<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::ExprKind::Ident(id) = expr.kind {
            self.mark_symbol_used(id.name);
        }

        self.walk_expr(expr)
    }

    fn visit_path(&mut self, path: &'ast ast::PathSlice) -> ControlFlow<Self::BreakValue> {
        for id in path.segments() {
            self.mark_symbol_used(id.name);
        }

        self.walk_path(path)
    }

    fn visit_ty(&mut self, ty: &'ast ast::Type<'ast>) -> ControlFlow<Self::BreakValue> {
        if let ast::TypeKind::Custom(path) = &ty.kind {
            self.mark_symbol_used(path.first().name);
        }

        self.walk_ty(ty)
    }
}
