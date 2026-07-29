//! Type-aware filtering for binary operator mutations.
//!
//! This module records replacements that have the same result as the original comparison for
//! every value in an operand's type range. It deliberately does not filter replacements merely
//! because they are tautological. For example, `uint256 x == 0` and `x <= 0` are equivalent, but
//! `x < 0` is retained because changing the original condition to `false` can expose missing tests.

use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use alloy_primitives::U256;
use eyre::Result;
use foundry_cli::opts::configure_pcx_from_compile_output;
use foundry_compilers::{ProjectCompileOutput, compilers::multi::MultiCompiler};
use foundry_config::Config;
use solar::{
    ast::{BinOpKind, LitKind, UnOpKind},
    interface::{Session, source_map::FileName},
    sema::{
        Compiler, Gcx,
        hir::{self, ElementaryType, ExprKind, TypeKind, Visit},
        ty::TyKind,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EquivalentMutation {
    lo: u32,
    hi: u32,
    new_op: BinOpKind,
}

impl EquivalentMutation {
    pub fn new(span: solar::ast::Span, new_op: BinOpKind) -> Self {
        Self { lo: span.lo().0, hi: span.hi().0, new_op }
    }
}

pub type EquivalentMutationSet = HashSet<EquivalentMutation>;
pub type EquivalentMutationsByPath = HashMap<PathBuf, EquivalentMutationSet>;

pub fn collect_equivalent_mutations(
    config: &Config,
    output: &ProjectCompileOutput<MultiCompiler>,
) -> Result<EquivalentMutationsByPath> {
    let mut compiler = Compiler::new(Session::builder().with_silent_emitter(None).build());
    compiler.enter_mut(|compiler| {
        let mut pcx = compiler.parse();
        configure_pcx_from_compile_output(&mut pcx, config, output, None)?;
        pcx.parse();

        let Ok(ControlFlow::Continue(())) = compiler.lower_asts() else {
            return Ok(EquivalentMutationsByPath::new());
        };
        let _ = compiler.analysis();
        Ok(collect_from_gcx(compiler.gcx()))
    })
}

fn collect_from_gcx<'gcx>(gcx: Gcx<'gcx>) -> EquivalentMutationsByPath {
    let mut by_path = EquivalentMutationsByPath::new();
    for source_id in gcx.hir.source_ids() {
        let source = gcx.hir.source(source_id);
        let FileName::Real(path) = &source.file.name else { continue };
        let mut collector = EquivalentMutationCollector {
            gcx,
            // HIR uses offsets in the compiler-wide source map, while the mutation visitor parses
            // each target independently and uses offsets relative to that file.
            source_start: source.file.start_pos.0,
            mutations: EquivalentMutationSet::new(),
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

struct EquivalentMutationCollector<'hir> {
    gcx: Gcx<'hir>,
    source_start: u32,
    mutations: EquivalentMutationSet,
}

impl<'hir> Visit<'hir> for EquivalentMutationCollector<'hir> {
    type BreakValue = ();

    fn hir(&self) -> &'hir hir::Hir<'hir> {
        &self.gcx.hir
    }

    fn visit_expr(&mut self, expr: &'hir hir::Expr<'hir>) -> ControlFlow<Self::BreakValue> {
        if let ExprKind::Binary(left, op, right) = &expr.kind
            && op.kind.is_cmp()
        {
            self.collect_comparison(expr, left, op.kind, right);
        }
        self.walk_expr(expr)
    }
}

impl EquivalentMutationCollector<'_> {
    fn collect_comparison(
        &mut self,
        expr: &hir::Expr<'_>,
        left: &hir::Expr<'_>,
        original: BinOpKind,
        right: &hir::Expr<'_>,
    ) {
        let comparison = if let (Some(range), Some(value)) =
            (integer_range(self.gcx, left), constant_value(right))
        {
            Some((range, value, original, false))
        } else if let (Some(value), Some(range)) =
            (constant_value(left), integer_range(self.gcx, right))
        {
            Some((range, value, flip(original), true))
        } else {
            None
        };
        let Some((range, value, normalized_original, operands_reversed)) = comparison else {
            return;
        };

        for candidate in [
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
        ] {
            if candidate == original {
                continue;
            }
            let normalized_candidate = if operands_reversed { flip(candidate) } else { candidate };
            if equivalent_at_boundary(range, value, normalized_original, normalized_candidate) {
                let Some(lo) = expr.span.lo().0.checked_sub(self.source_start) else { continue };
                let Some(hi) = expr.span.hi().0.checked_sub(self.source_start) else { continue };
                let span = solar::ast::Span::new(
                    solar::interface::BytePos(lo),
                    solar::interface::BytePos(hi),
                );
                self.mutations.insert(EquivalentMutation::new(span, candidate));
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IntegerRange {
    lower: SignedValue,
    upper: SignedValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SignedValue {
    negative: bool,
    magnitude: U256,
}

impl SignedValue {
    const ZERO: Self = Self { negative: false, magnitude: U256::ZERO };

    fn new(negative: bool, magnitude: U256) -> Self {
        if magnitude.is_zero() { Self::ZERO } else { Self { negative, magnitude } }
    }
}

fn integer_range(gcx: Gcx<'_>, expr: &hir::Expr<'_>) -> Option<IntegerRange> {
    if let Some(ty) = gcx.type_of_expr(expr.id)
        && let TyKind::Elementary(ty) = ty.peel_refs().kind
    {
        return integer_range_for_type(ty);
    }

    // Fall back to types available during lowering when Solar could not infer this expression.
    let ty = match &expr.peel_parens().kind {
        ExprKind::Ident(_) => {
            let variable = expr.as_variable()?;
            match gcx.hir.variable(variable).ty.kind {
                TypeKind::Elementary(ty) => ty,
                _ => return None,
            }
        }
        ExprKind::Call(callee, args, _) => {
            let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) =
                &callee.peel_parens().kind
            else {
                return None;
            };
            let mut expressions = args.exprs();
            expressions.next()?;
            if expressions.next().is_some() {
                return None;
            }
            *ty
        }
        _ => return None,
    };
    integer_range_for_type(ty)
}

fn integer_range_for_type(ty: ElementaryType) -> Option<IntegerRange> {
    match ty {
        ElementaryType::UInt(size) => {
            let bits = size.bits();
            let upper =
                if bits == 256 { U256::MAX } else { (U256::from(1u8) << bits) - U256::from(1u8) };
            Some(IntegerRange { lower: SignedValue::ZERO, upper: SignedValue::new(false, upper) })
        }
        ElementaryType::Int(size) => {
            let half = U256::from(1u8) << (size.bits() - 1);
            Some(IntegerRange {
                lower: SignedValue::new(true, half),
                upper: SignedValue::new(false, half - U256::from(1u8)),
            })
        }
        ElementaryType::Address(_) => {
            let upper = (U256::from(1u8) << 160) - U256::from(1u8);
            Some(IntegerRange { lower: SignedValue::ZERO, upper: SignedValue::new(false, upper) })
        }
        _ => None,
    }
}

fn constant_value(expr: &hir::Expr<'_>) -> Option<SignedValue> {
    match &expr.peel_parens().kind {
        ExprKind::Lit(lit) => match lit.kind {
            LitKind::Number(value) => Some(SignedValue::new(false, value)),
            LitKind::Address(value) => {
                Some(SignedValue::new(false, U256::from_be_slice(value.as_slice())))
            }
            _ => None,
        },
        ExprKind::Unary(op, inner) if op.kind == UnOpKind::Neg => {
            let ExprKind::Lit(lit) = &inner.peel_parens().kind else { return None };
            let LitKind::Number(value) = lit.kind else { return None };
            Some(SignedValue::new(true, value))
        }
        ExprKind::Member(type_call, member) => {
            let ExprKind::TypeCall(hir::Type { kind: TypeKind::Elementary(ty), .. }) =
                &type_call.peel_parens().kind
            else {
                return None;
            };
            let range = integer_range_for_type(*ty)?;
            match member.as_str() {
                "min" => Some(range.lower),
                "max" => Some(range.upper),
                _ => None,
            }
        }
        ExprKind::Call(callee, args, _) => {
            let ExprKind::Type(hir::Type { kind: TypeKind::Elementary(ty), .. }) =
                &callee.peel_parens().kind
            else {
                return None;
            };
            let mut expressions = args.exprs();
            let inner = expressions.next()?;
            if expressions.next().is_some() {
                return None;
            }
            let range = integer_range_for_type(*ty)?;
            let value = constant_value(inner)?;
            (value == range.lower || value == range.upper).then_some(value)
        }
        _ => None,
    }
}

fn equivalent_at_boundary(
    range: IntegerRange,
    value: SignedValue,
    original: BinOpKind,
    candidate: BinOpKind,
) -> bool {
    let pair = (original, candidate);
    if value == range.lower {
        matches!(
            pair,
            (BinOpKind::Eq, BinOpKind::Le)
                | (BinOpKind::Le, BinOpKind::Eq)
                | (BinOpKind::Ne, BinOpKind::Gt)
                | (BinOpKind::Gt, BinOpKind::Ne)
        )
    } else if value == range.upper {
        matches!(
            pair,
            (BinOpKind::Eq, BinOpKind::Ge)
                | (BinOpKind::Ge, BinOpKind::Eq)
                | (BinOpKind::Ne, BinOpKind::Lt)
                | (BinOpKind::Lt, BinOpKind::Ne)
        )
    } else {
        false
    }
}

const fn flip(op: BinOpKind) -> BinOpKind {
    match op {
        BinOpKind::Lt => BinOpKind::Gt,
        BinOpKind::Le => BinOpKind::Ge,
        BinOpKind::Gt => BinOpKind::Lt,
        BinOpKind::Ge => BinOpKind::Le,
        BinOpKind::Eq | BinOpKind::Ne => op,
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solar::interface::BytePos;

    fn collect(source: &str) -> EquivalentMutationSet {
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

    fn mutation(source: &str, expression: &str, new_op: BinOpKind) -> EquivalentMutation {
        let lo = source.find(expression).expect("expression") as u32;
        EquivalentMutation::new(
            solar::ast::Span::new(BytePos(lo), BytePos(lo + expression.len() as u32)),
            new_op,
        )
    }

    #[test]
    fn identifies_only_equivalent_unsigned_lower_bound_mutations() {
        let range =
            integer_range_for_type(ElementaryType::UInt(solar::ast::TypeSize::new_int_bits(256)))
                .unwrap();

        assert!(equivalent_at_boundary(range, SignedValue::ZERO, BinOpKind::Eq, BinOpKind::Le));
        assert!(!equivalent_at_boundary(range, SignedValue::ZERO, BinOpKind::Eq, BinOpKind::Lt));
        assert!(equivalent_at_boundary(range, SignedValue::ZERO, BinOpKind::Ne, BinOpKind::Gt));
    }

    #[test]
    fn does_not_treat_zero_as_a_signed_boundary() {
        let range =
            integer_range_for_type(ElementaryType::Int(solar::ast::TypeSize::new_int_bits(256)))
                .unwrap();

        assert!(!equivalent_at_boundary(range, SignedValue::ZERO, BinOpKind::Eq, BinOpKind::Le));
    }

    #[test]
    fn collects_typed_unsigned_boundary_equivalents() {
        let source = r#"
contract Test {
    function check(uint256 x) external pure returns (bool) {
        return x == 0;
    }
}
"#;
        let mutations = collect(source);

        assert!(mutations.contains(&mutation(source, "x == 0", BinOpKind::Le)));
        assert!(!mutations.contains(&mutation(source, "x == 0", BinOpKind::Lt)));
    }

    #[test]
    fn preserves_signed_zero_mutations() {
        let source = r#"
contract Test {
    function check(int256 x) external pure returns (bool) {
        return x == 0;
    }
}
"#;
        let mutations = collect(source);

        assert!(!mutations.contains(&mutation(source, "x == 0", BinOpKind::Le)));
    }

    #[test]
    fn handles_reversed_operands_and_upper_bounds() {
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

        assert!(mutations.contains(&mutation(source, "0 == x", BinOpKind::Ge)));
        assert!(!mutations.contains(&mutation(source, "0 == x", BinOpKind::Gt)));
        assert!(mutations.contains(&mutation(source, "x == type(uint8).max", BinOpKind::Ge)));
    }

    #[test]
    fn uses_inferred_member_types_and_typed_zero_constants() {
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

        assert!(mutations.contains(&mutation(source, "value.length == 0", BinOpKind::Le)));
        assert!(mutations.contains(&mutation(source, "value == address(0)", BinOpKind::Le)));
    }
}
