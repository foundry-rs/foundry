//! Type-aware filtering for operator mutations.
//!
//! This module records replacements that are type-invalid. It deliberately does not filter
//! replacements merely because they return the same value for every input: different operators can
//! still compile to gas-distinct bytecode.

use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use eyre::Result;
use foundry_cli::opts::configure_pcx_from_compile_output;
use foundry_compilers::{ProjectCompileOutput, compilers::multi::MultiCompiler};
use foundry_config::Config;
use solar::{
    ast::{BinOpKind, UnOpKind},
    interface::{Session, source_map::FileName},
    sema::{
        Compiler, Gcx,
        hir::{self, ElementaryType, ExprKind, Visit},
        ty::TyKind,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ReplacementOperator {
    Binary(BinOpKind),
    Unary(UnOpKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MutationExclusion {
    lo: u32,
    hi: u32,
    new_op: ReplacementOperator,
}

impl MutationExclusion {
    pub fn binary(span: solar::ast::Span, new_op: BinOpKind) -> Self {
        Self { lo: span.lo().0, hi: span.hi().0, new_op: ReplacementOperator::Binary(new_op) }
    }

    pub fn unary(span: solar::ast::Span, new_op: UnOpKind) -> Self {
        Self { lo: span.lo().0, hi: span.hi().0, new_op: ReplacementOperator::Unary(new_op) }
    }
}

pub type MutationExclusionSet = HashSet<MutationExclusion>;
pub type MutationExclusionsByPath = HashMap<PathBuf, MutationExclusionSet>;

pub fn collect_mutation_exclusions(
    config: &Config,
    output: &ProjectCompileOutput<MultiCompiler>,
) -> Result<MutationExclusionsByPath> {
    let mut compiler = Compiler::new(Session::builder().with_silent_emitter(None).build());
    compiler.enter_mut(|compiler| {
        let mut pcx = compiler.parse();
        configure_pcx_from_compile_output(&mut pcx, config, output, None)?;
        pcx.parse();

        let Ok(ControlFlow::Continue(())) = compiler.lower_asts() else {
            return Ok(MutationExclusionsByPath::new());
        };
        let _ = compiler.analysis();
        Ok(collect_from_gcx(compiler.gcx()))
    })
}

fn collect_from_gcx<'gcx>(gcx: Gcx<'gcx>) -> MutationExclusionsByPath {
    let mut by_path = MutationExclusionsByPath::new();
    for source_id in gcx.hir.source_ids() {
        let source = gcx.hir.source(source_id);
        let FileName::Real(path) = &source.file.name else { continue };
        let mut collector = MutationExclusionCollector {
            gcx,
            // HIR uses offsets in the compiler-wide source map, while the mutation visitor parses
            // each target independently and uses offsets relative to that file.
            source_start: source.file.start_pos.0,
            mutations: MutationExclusionSet::new(),
        };
        let _ = collector.visit_nested_source(source_id);
        if !collector.mutations.is_empty() {
            by_path.insert(normalize_path(path), collector.mutations);
        }
    }
    by_path
}

pub fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

struct MutationExclusionCollector<'hir> {
    gcx: Gcx<'hir>,
    source_start: u32,
    mutations: MutationExclusionSet,
}

impl<'hir> Visit<'hir> for MutationExclusionCollector<'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Binary(left, op, right) = &expr.kind
            && (op.kind.is_cmp() || matches!(op.kind, BinOpKind::And | BinOpKind::Or))
        {
            self.collect_comparison(expr, left, op.kind, right);
        }
        if let ExprKind::Unary(_, operand) = &expr.kind
            && is_unsigned(self.gcx, operand)
            && let Some(span) = self.local_span(expr.span)
        {
            self.mutations.insert(MutationExclusion::unary(span, UnOpKind::Neg));
        }
        self.walk_expr(expr)
    }
}

impl MutationExclusionCollector<'_> {
    fn collect_comparison(
        &mut self,
        expr: &hir::Expr<'_>,
        left: &hir::Expr<'_>,
        original: BinOpKind,
        right: &hir::Expr<'_>,
    ) {
        let Some(span) = self.local_span(expr.span) else { return };

        for candidate in [
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
            BinOpKind::Or,
            BinOpKind::And,
        ] {
            if candidate != original
                && is_type_invalid_replacement(self.gcx, left, original, right, candidate)
            {
                self.mutations.insert(MutationExclusion::binary(span, candidate));
            }
        }
    }

    fn local_span(&self, span: solar::ast::Span) -> Option<solar::ast::Span> {
        let lo = span.lo().0.checked_sub(self.source_start)?;
        let hi = span.hi().0.checked_sub(self.source_start)?;
        Some(solar::ast::Span::new(solar::interface::BytePos(lo), solar::interface::BytePos(hi)))
    }
}

fn is_type_invalid_replacement(
    gcx: Gcx<'_>,
    left: &hir::Expr<'_>,
    original: BinOpKind,
    right: &hir::Expr<'_>,
    candidate: BinOpKind,
) -> bool {
    match candidate {
        BinOpKind::And | BinOpKind::Or => matches!(
            comparison_operand_kind(gcx, left, right),
            Some(ComparisonOperandKind::Function | ComparisonOperandKind::Other)
        ),
        BinOpKind::Lt | BinOpKind::Le | BinOpKind::Gt | BinOpKind::Ge => {
            matches!(original, BinOpKind::And | BinOpKind::Or)
                || matches!(
                    comparison_operand_kind(gcx, left, right),
                    Some(ComparisonOperandKind::Bool | ComparisonOperandKind::Function)
                )
        }
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComparisonOperandKind {
    Bool,
    Function,
    Other,
}

fn comparison_operand_kind(
    gcx: Gcx<'_>,
    left: &hir::Expr<'_>,
    right: &hir::Expr<'_>,
) -> Option<ComparisonOperandKind> {
    let left = gcx.type_of_expr(left.id)?;
    let right = gcx.type_of_expr(right.id)?;
    match (left.peel_refs().kind, right.peel_refs().kind) {
        (TyKind::Elementary(ElementaryType::Bool), TyKind::Elementary(ElementaryType::Bool)) => {
            Some(ComparisonOperandKind::Bool)
        }
        (TyKind::Fn(_), TyKind::Fn(_)) => Some(ComparisonOperandKind::Function),
        (TyKind::Err(_), _) | (_, TyKind::Err(_)) => None,
        _ => Some(ComparisonOperandKind::Other),
    }
}

fn is_unsigned(gcx: Gcx<'_>, expr: &hir::Expr<'_>) -> bool {
    gcx.type_of_expr(expr.peel_parens().id).is_some_and(|ty| {
        matches!(ty.peel_refs().kind, TyKind::Elementary(ElementaryType::UInt(_)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar::interface::BytePos;

    fn collect(source: &str) -> MutationExclusionSet {
        let path = PathBuf::from("Test.sol");
        let mut compiler = Compiler::new(Session::builder().with_silent_emitter(None).build());
        compiler.enter_mut(|compiler| {
            let mut pcx = compiler.parse();
            let file =
                pcx.sess.source_map().new_source_file(path.clone(), source).expect("source file");
            pcx.add_file(file);
            pcx.parse();
            assert!(matches!(compiler.lower_asts(), Ok(ControlFlow::Continue(()))));
            let _ = compiler.analysis();
            collect_from_gcx(compiler.gcx()).remove(&path).unwrap_or_default()
        })
    }

    fn mutation(source: &str, expression: &str, new_op: BinOpKind) -> MutationExclusion {
        let lo = source.find(expression).expect("expression") as u32;
        MutationExclusion::binary(
            solar::ast::Span::new(BytePos(lo), BytePos(lo + expression.len() as u32)),
            new_op,
        )
    }

    fn unary_mutation(source: &str, expression: &str, new_op: UnOpKind) -> MutationExclusion {
        let lo = source.find(expression).expect("expression") as u32;
        MutationExclusion::unary(
            solar::ast::Span::new(BytePos(lo), BytePos(lo + expression.len() as u32)),
            new_op,
        )
    }

    #[test]
    fn preserves_gas_distinct_unsigned_boundary_mutations() {
        let source = r#"
contract Test {
    function check(uint256 x) external pure returns (bool) {
        return x == 0;
    }
}
"#;
        let mutations = collect(source);

        assert!(!mutations.contains(&mutation(source, "x == 0", BinOpKind::Le)));
        assert!(!mutations.contains(&mutation(source, "x == 0", BinOpKind::Lt)));
    }

    #[test]
    fn excludes_negation_only_for_unsigned_unary_operands() {
        let source = r#"
contract Test {
    function check(uint256 unsigned, int256 signed) external pure {
        unsigned++;
        ++unsigned;
        signed++;
    }
}
"#;
        let mutations = collect(source);

        assert!(mutations.contains(&unary_mutation(source, "unsigned++", UnOpKind::Neg)));
        assert!(mutations.contains(&unary_mutation(source, "++unsigned", UnOpKind::Neg)));
        assert!(!mutations.contains(&unary_mutation(source, "signed++", UnOpKind::Neg)));
    }

    #[test]
    fn preserves_overloaded_negation_for_unsigned_udvt() {
        let source = r#"
type Amount is uint256;

function negate(Amount amount) pure returns (Amount) {
    return Amount.wrap(type(uint256).max - Amount.unwrap(amount));
}

function complement(Amount amount) pure returns (Amount) {
    return Amount.wrap(~Amount.unwrap(amount));
}

using {negate as -, complement as ~} for Amount global;

contract Test {
    function check(Amount amount) external pure returns (Amount) {
        return ~amount;
    }
}
"#;
        let mutations = collect(source);

        assert!(!mutations.contains(&unary_mutation(source, "~amount", UnOpKind::Neg)));
    }

    #[test]
    fn preserves_negation_for_unresolved_operand() {
        let source = r#"
contract Test {
    function check() external pure {
        unresolved++;
    }
}
"#;
        let mutations = collect(source);

        assert!(!mutations.contains(&unary_mutation(source, "unresolved++", UnOpKind::Neg)));
    }

    #[test]
    fn excludes_logical_replacements_for_numeric_comparisons() {
        let source = r#"
contract Test {
    function check(uint256 left, uint256 right) external pure returns (bool) {
        return left == right;
    }
}
"#;
        let mutations = collect(source);

        assert!(mutations.contains(&mutation(source, "left == right", BinOpKind::And)));
        assert!(mutations.contains(&mutation(source, "left == right", BinOpKind::Or)));
        assert!(!mutations.contains(&mutation(source, "left == right", BinOpKind::Lt)));
        assert!(!mutations.contains(&mutation(source, "left == right", BinOpKind::Ne)));
    }

    #[test]
    fn excludes_ordered_replacements_for_boolean_equality() {
        let source = r#"
contract Test {
    function check(bool left, bool right) external pure returns (bool) {
        return left == right;
    }
}
"#;
        let mutations = collect(source);

        for candidate in [BinOpKind::Lt, BinOpKind::Le, BinOpKind::Gt, BinOpKind::Ge] {
            assert!(mutations.contains(&mutation(source, "left == right", candidate)));
        }
        for candidate in [BinOpKind::Ne, BinOpKind::And, BinOpKind::Or] {
            assert!(!mutations.contains(&mutation(source, "left == right", candidate)));
        }
    }

    #[test]
    fn excludes_ordered_replacements_for_logical_operations() {
        let source = r#"
contract Test {
    function check(bool left, bool right) external pure returns (bool) {
        return left && right;
    }
}
"#;
        let mutations = collect(source);

        for candidate in [BinOpKind::Lt, BinOpKind::Le, BinOpKind::Gt, BinOpKind::Ge] {
            assert!(mutations.contains(&mutation(source, "left && right", candidate)));
        }
        for candidate in [BinOpKind::Eq, BinOpKind::Ne, BinOpKind::Or] {
            assert!(!mutations.contains(&mutation(source, "left && right", candidate)));
        }
    }

    #[test]
    fn excludes_non_equality_replacements_for_function_comparisons() {
        let source = r#"
contract Test {
    function check(
        function() external left,
        function() external right
    ) external pure returns (bool) {
        return left == right;
    }
}
"#;
        let mutations = collect(source);

        for candidate in [
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::And,
            BinOpKind::Or,
        ] {
            assert!(mutations.contains(&mutation(source, "left == right", candidate)));
        }
        assert!(!mutations.contains(&mutation(source, "left == right", BinOpKind::Ne)));
    }

    #[test]
    fn preserves_reversed_and_upper_boundary_mutations() {
        let source = r#"
contract Test {
    function lower(uint256 x) external pure returns (bool) {
        return 0 == x;
    }

    function upper(uint8 x) external pure returns (bool) {
        return x == type(uint8).max;
    }
}
"#;
        let mutations = collect(source);

        assert!(!mutations.contains(&mutation(source, "0 == x", BinOpKind::Ge)));
        assert!(!mutations.contains(&mutation(source, "0 == x", BinOpKind::Gt)));
        assert!(!mutations.contains(&mutation(source, "x == type(uint8).max", BinOpKind::Ge)));
    }

    #[test]
    fn preserves_member_and_typed_constant_boundary_mutations() {
        let source = r#"
contract Test {
    function empty(bytes memory value) external pure returns (bool) {
        return value.length == 0;
    }

    function zero(address value) external pure returns (bool) {
        return value == address(0);
    }
}
"#;
        let mutations = collect(source);

        assert!(!mutations.contains(&mutation(source, "value.length == 0", BinOpKind::Le)));
        assert!(!mutations.contains(&mutation(source, "value == address(0)", BinOpKind::Le)));
    }
}
