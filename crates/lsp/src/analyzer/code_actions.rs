use super::Analyzer;
use solar_interface::{BytePos, Span, source_map::FileName};
use solar_sema::{
    hir::{self, Visit},
    ty::{Gcx, TyKind},
};
use std::ops::ControlFlow;
use tower_lsp::lsp_types::{Location, Position, Range, Url};

impl Analyzer {
    /// Finds the definition of the symbol at the given location.
    pub fn goto_definition(&self, uri: &Url, position: Position) -> Option<Location> {
        self.sess.enter(|| {
            let find_position_span =
                |gcx: Gcx<'_>, file_path: std::path::PathBuf| -> Option<Span> {
                    for id in gcx.hir.source_ids() {
                        let source_file = &gcx.hir.source(id).file;
                        if let FileName::Real(ref path) = source_file.name
                            && path == &file_path
                        {
                            let byte_offset = crate::utils::position_to_byte_offset(
                                &source_file.src,
                                position.line,
                                position.character,
                            );
                            let pos = source_file.start_pos + byte_offset as u32;
                            let mut visitor = DefinitionVisitor { gcx, pos };
                            if let ControlFlow::Break(span) = visitor.visit_nested_source(id) {
                                return Some(span);
                            }
                        }
                    }
                    None
                };

            let gcx = self.ogcxw.as_ref().unwrap().gcx_wrapper().get();
            let source_map = gcx.sess.source_map();
            let file_path =
                uri.to_file_path().ok().and_then(|path| dunce::canonicalize(path).ok())?;

            let target_span = find_position_span(gcx, file_path)?;
            let (file, start_line, start_char, end_line, end_char) =
                source_map.span_to_location_info(target_span);

            let file = file?;
            let target_uri = if let FileName::Real(path) = &file.name { Some(path) } else { None }
                .and_then(|p| Url::from_file_path(p).ok())?;

            Some(Location {
                uri: target_uri,
                range: Range {
                    start: Position {
                        line: start_line.saturating_sub(1) as u32,
                        character: start_char.saturating_sub(1) as u32,
                    },
                    end: Position {
                        line: end_line.saturating_sub(1) as u32,
                        character: end_char.saturating_sub(1) as u32,
                    },
                },
            })
        })
    }
}

/// A visitor to find the definition of a symbol at a given position.
struct DefinitionVisitor<'gcx> {
    gcx: solar_sema::ty::Gcx<'gcx>,
    pos: BytePos,
}

impl<'gcx> DefinitionVisitor<'gcx> {
    /// Helper to get the span for an item's name, falling back to the item's full span.
    fn get_item_name_span(&self, item_id: hir::ItemId) -> Span {
        let item = self.gcx.hir.item(item_id);
        match item {
            hir::Item::Variable(var) => var.name.map_or(var.span, |name| name.span),
            hir::Item::Function(func) => func.name.map_or(func.span, |name| name.span),
            _ => item.name().map_or_else(|| item.span(), |name| name.span),
        }
    }

    /// Recursively visit a type to find a definition.
    fn visit_type(&self, ty: &'gcx hir::Type<'gcx>) -> ControlFlow<Span> {
        if !ty.span.contains_pos(self.pos) {
            return ControlFlow::Continue(());
        }

        match &ty.kind {
            hir::TypeKind::Array(array_type) => self.visit_type(&array_type.element),
            hir::TypeKind::Custom(item_id) => ControlFlow::Break(self.get_item_name_span(*item_id)),
            _ => ControlFlow::Continue(()),
        }
    }
}

impl<'hir> Visit<'hir> for DefinitionVisitor<'hir> {
    type BreakValue = Span;

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Span> {
        if !expr.span.contains_pos(self.pos) {
            return ControlFlow::Continue(());
        }

        if let ControlFlow::Break(span) = self.walk_expr(expr) {
            return ControlFlow::Break(span);
        }

        match &expr.kind {
            hir::ExprKind::Ident(res) => {
                if let Some(hir::Res::Item(id)) = res.first() {
                    return ControlFlow::Break(self.get_item_name_span(*id));
                }
            }
            hir::ExprKind::New(ty) | hir::ExprKind::Type(ty) | hir::ExprKind::TypeCall(ty) => {
                return self.visit_type(ty);
            }
            // For member calls, resolve the base expression first, and then the member indent.
            hir::ExprKind::Member(base_expr, member_ident) => {
                if let hir::ExprKind::Ident(res) = &base_expr.kind {
                    if let Some(base_res) = res.first() {
                        let base_ty = self.gcx.type_of_res(*base_res);
                        if let TyKind::Contract(contract_id) = base_ty.kind {
                            for item_id in self.gcx.hir.contract_item_ids(contract_id) {
                                if self.gcx.item_name_opt(item_id) == Some(*member_ident) {
                                    let span = self.get_item_name_span(item_id);
                                    return ControlFlow::Break(span);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }

        ControlFlow::Continue(())
    }
}

trait SpanExt {
    fn contains_pos(&self, pos: BytePos) -> bool;
}

impl SpanExt for Span {
    fn contains_pos(&self, pos: BytePos) -> bool {
        self.lo() <= pos && pos < self.hi()
    }
}

#[cfg(test)]
mod tests {
    use crate::analyzer::test_utils::setup_analyzer;
    use tower_lsp::lsp_types::*;

    const GOTO_DEF: &str = include_str!("../../testdata/src/GoToDef.sol");
    const DEPS: &str = include_str!("../../testdata/src/Deps.sol");

    #[test]
    fn test_go_to_definition_state_variable() {
        let (uri, analyzer, _temp_dir) =
            setup_analyzer(&[("GoToDef.sol", GOTO_DEF), ("Deps.sol", DEPS)]);

        // usage of `another` in `anotehr.add_num(1);`
        let location = analyzer.goto_definition(&uri, Position { line: 13, character: 10 });

        assert!(location.is_some(), "Expected to find a definition");
        let location = location.unwrap();
        assert_eq!(location.uri, uri, "Definition is in the wrong file");

        // definition of `another` in `AnotherContract public another;`
        assert_eq!(
            location.range,
            Range {
                start: Position { line: 8, character: 27 },
                end: Position { line: 8, character: 34 },
            },
            "Definition has the wrong range"
        );
    }

    #[test]
    fn test_go_to_definition_imported_function() {
        let (uri, analyzer, _temp_dir) =
            setup_analyzer(&[("GoToDef.sol", GOTO_DEF), ("Deps.sol", DEPS)]);

        // usage of `add_num` in `another.add_num(1);`
        let location = analyzer.goto_definition(&uri, Position { line: 13, character: 21 });

        assert!(location.is_some(), "Expected to find a definition for imported function");
        let location = location.unwrap();
        let expected_uri = dunce::canonicalize(_temp_dir.path().join("src/Deps.sol"))
            .ok()
            .and_then(|path| Url::from_file_path(path).ok())
            .unwrap();
        assert_eq!(location.uri, expected_uri, "Definition is in the wrong file");

        // definition of `function add_num()` in `A.sol`
        assert_eq!(
            location.range,
            Range {
                start: Position { line: 6, character: 13 },
                end: Position { line: 6, character: 20 },
            },
            "Definition has the wrong range"
        );
    }
}
