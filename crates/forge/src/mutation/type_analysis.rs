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
    ast::{BinOpKind, LitKind, UnOpKind},
    interface::{Session, source_map::FileName},
    sema::{
        Compiler, CompilerRef, Gcx,
        hir::{self, ElementaryType, ExprKind, Visit},
        ty::TyKind,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ReplacementOperator {
    Assignment(AssignmentReplacement),
    Binary(BinOpKind),
    Unary(UnOpKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignmentReplacement {
    Zero,
    Negate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MutationExclusion {
    lo: u32,
    hi: u32,
    new_op: ReplacementOperator,
}

impl MutationExclusion {
    pub fn assignment(span: solar::ast::Span, replacement: AssignmentReplacement) -> Self {
        Self {
            lo: span.lo().0,
            hi: span.hi().0,
            new_op: ReplacementOperator::Assignment(replacement),
        }
    }

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
        Ok(analyze_and_collect(compiler))
    })
}

fn analyze_and_collect(compiler: &CompilerRef<'_>) -> MutationExclusionsByPath {
    let Ok(ControlFlow::Continue(())) = compiler.analysis() else {
        return MutationExclusionsByPath::new();
    };
    if compiler.dcx().has_errors().is_err() {
        return MutationExclusionsByPath::new();
    }
    collect_from_gcx(compiler.gcx())
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
        if let ExprKind::Assign(destination, op, value) = &expr.kind
            && let Some(ty) = self.gcx.type_of_expr(destination.id)
        {
            if let Some(op) = op {
                self.collect_compound_assignment(value, ty, op.kind);
            } else {
                self.collect_assignment(value, ty);
            }
        }
        if let ExprKind::Binary(left, op, right) = &expr.kind
            && (op.kind.is_cmp() || matches!(op.kind, BinOpKind::And | BinOpKind::Or))
        {
            self.collect_comparison(expr, left, op.kind, right);
        }
        if let ExprKind::Unary(_, operand) = &expr.kind
            && let Some(kind) = unary_operand_kind(self.gcx, operand)
            && let Some(span) = self.local_span(expr.span)
        {
            match kind {
                UnaryOperandKind::SignedInteger => {}
                UnaryOperandKind::UnsignedInteger => {
                    self.mutations.insert(MutationExclusion::unary(span, UnOpKind::Neg));
                }
                UnaryOperandKind::FixedBytes => {
                    for op in [
                        UnOpKind::PreInc,
                        UnOpKind::PreDec,
                        UnOpKind::PostInc,
                        UnOpKind::PostDec,
                        UnOpKind::Neg,
                    ] {
                        self.mutations.insert(MutationExclusion::unary(span, op));
                    }
                }
            }
        }
        if let ExprKind::Unary(_, operand) = &expr.kind
            && is_non_storage_push_call(self.gcx, operand)
            && let Some(span) = self.local_span(expr.span)
        {
            for op in [UnOpKind::PreInc, UnOpKind::PreDec, UnOpKind::PostInc, UnOpKind::PostDec] {
                self.mutations.insert(MutationExclusion::unary(span, op));
            }
        }
        self.walk_expr(expr)
    }

    fn visit_var(&mut self, var: &'hir hir::Variable<'hir>) -> ControlFlow<Self::BreakValue> {
        if let Some(value) = var.initializer {
            self.collect_assignment(value, self.gcx.type_of_hir_ty(&var.ty));
        }
        self.walk_var(var)
    }
}

impl MutationExclusionCollector<'_> {
    fn collect_assignment(&mut self, value: &hir::Expr<'_>, destination: solar::sema::ty::Ty<'_>) {
        let Some(span) = self.local_span(value.span) else { return };
        let Some(kind) = assignment_destination_kind(destination) else { return };

        if !kind.accepts_zero() {
            self.mutations.insert(MutationExclusion::assignment(span, AssignmentReplacement::Zero));
        }
        if !kind.accepts_negation() {
            self.mutations
                .insert(MutationExclusion::assignment(span, AssignmentReplacement::Negate));
        }
    }

    fn collect_compound_assignment(
        &mut self,
        value: &hir::Expr<'_>,
        destination: solar::sema::ty::Ty<'_>,
        op: BinOpKind,
    ) {
        let Some(span) = self.local_span(value.span) else { return };
        let Some(destination) = assignment_destination_kind(destination) else { return };
        let Some(value_accepts_negation) = assignment_value_accepts_negation(self.gcx, value)
        else {
            return;
        };

        if op.is_shift() || !destination.accepts_negation() || !value_accepts_negation {
            self.mutations
                .insert(MutationExclusion::assignment(span, AssignmentReplacement::Negate));
        }
    }

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

#[derive(Clone, Copy)]
enum AssignmentDestinationKind {
    SignedNumber,
    UnsignedNumber,
    FixedBytes,
    Other,
}

impl AssignmentDestinationKind {
    const fn accepts_zero(self) -> bool {
        !matches!(self, Self::Other)
    }

    const fn accepts_negation(self) -> bool {
        matches!(self, Self::SignedNumber)
    }
}

fn assignment_destination_kind(ty: solar::sema::ty::Ty<'_>) -> Option<AssignmentDestinationKind> {
    match ty.peel_refs().kind {
        TyKind::Elementary(ElementaryType::Int(_) | ElementaryType::Fixed(..)) => {
            Some(AssignmentDestinationKind::SignedNumber)
        }
        TyKind::Elementary(ElementaryType::UInt(_) | ElementaryType::UFixed(..)) => {
            Some(AssignmentDestinationKind::UnsignedNumber)
        }
        TyKind::Elementary(ElementaryType::FixedBytes(_)) => {
            Some(AssignmentDestinationKind::FixedBytes)
        }
        TyKind::Udvt(..) | TyKind::Err(_) => None,
        _ => Some(AssignmentDestinationKind::Other),
    }
}

fn assignment_value_accepts_negation(gcx: Gcx<'_>, value: &hir::Expr<'_>) -> Option<bool> {
    if matches!(&value.peel_parens().kind, ExprKind::Lit(lit) if matches!(lit.kind, LitKind::Number(_)))
    {
        return Some(true);
    }

    let ty = gcx.type_of_expr(value.peel_parens().id)?;
    match ty.peel_refs().kind {
        TyKind::Elementary(ElementaryType::Int(_) | ElementaryType::Fixed(..)) => Some(true),
        TyKind::Elementary(
            ElementaryType::UInt(_) | ElementaryType::UFixed(..) | ElementaryType::FixedBytes(_),
        ) => Some(false),
        TyKind::Udvt(..) | TyKind::Err(_) => None,
        _ => Some(false),
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

#[derive(Clone, Copy)]
enum UnaryOperandKind {
    SignedInteger,
    UnsignedInteger,
    FixedBytes,
}

fn unary_operand_kind(gcx: Gcx<'_>, expr: &hir::Expr<'_>) -> Option<UnaryOperandKind> {
    let ty = gcx.type_of_expr(expr.peel_parens().id)?;
    match ty.peel_refs().kind {
        TyKind::Elementary(ElementaryType::Int(_)) => Some(UnaryOperandKind::SignedInteger),
        TyKind::Elementary(ElementaryType::UInt(_)) => Some(UnaryOperandKind::UnsignedInteger),
        TyKind::Elementary(ElementaryType::FixedBytes(_)) => Some(UnaryOperandKind::FixedBytes),
        _ => None,
    }
}

fn is_non_storage_push_call(gcx: Gcx<'_>, expr: &hir::Expr<'_>) -> bool {
    let ExprKind::Call(callee, args, _) = &expr.peel_parens().kind else { return false };
    let ExprKind::Member(receiver, member) = &callee.peel_parens().kind else { return false };
    if member.as_str() != "push" || !args.is_empty() {
        return false;
    }

    !gcx.type_of_expr(receiver.id).is_some_and(|ty| {
        matches!(
            ty.kind,
            TyKind::Ref(inner, location)
                if location.is_storage()
                    && matches!(
                        inner.kind,
                        TyKind::DynArray(_) | TyKind::Elementary(ElementaryType::Bytes)
                    )
        )
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
            analyze_and_collect(compiler).remove(&path).unwrap_or_default()
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

    fn assignment_mutation(
        source: &str,
        value: &str,
        replacement: AssignmentReplacement,
    ) -> MutationExclusion {
        let lo = source.rfind(value).expect("value") as u32;
        MutationExclusion::assignment(
            solar::ast::Span::new(BytePos(lo), BytePos(lo + value.len() as u32)),
            replacement,
        )
    }

    #[test]
    fn excludes_assignments_invalid_for_destination_type() {
        let source = r#"
enum Choice { A, B }

contract Test {
    function check(address account, bool flag, uint256 unsigned, Choice choice) external pure {
        address accountCopy = account;
        bool flagCopy = flag;
        uint256 unsignedCopy = unsigned;
        Choice choiceCopy = choice;
    }
}
"#;
        let mutations = collect(source);

        for value in ["account", "flag", "choice"] {
            assert!(mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Zero,
            )));
            assert!(mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Negate,
            )));
        }
        assert!(!mutations.contains(&assignment_mutation(
            source,
            "unsigned",
            AssignmentReplacement::Zero,
        )));
        assert!(mutations.contains(&assignment_mutation(
            source,
            "unsigned",
            AssignmentReplacement::Negate,
        )));
    }

    #[test]
    fn uses_lhs_type_for_assignments() {
        let source = r#"
contract Test {
    function check(address account, bool flag) external pure {
        account = account;
        flag = flag;
    }
}
"#;
        let mutations = collect(source);

        for value in ["account", "flag"] {
            assert!(mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Zero,
            )));
            assert!(mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Negate,
            )));
        }
    }

    #[test]
    fn excludes_invalid_compound_assignment_negation() {
        let source = r#"
contract Test {
    uint256 unsignedTotal;
    int256 signedTotal;
    bytes32 word;

    function check(
        uint256 unsignedAmount,
        int256 signedAmount,
        bytes32 mask,
        uint8 shift
    ) external {
        unsignedTotal += unsignedAmount;
        signedTotal += signedAmount;
        word &= mask;
        signedTotal <<= shift;
        signedTotal += 11;
        signedTotal &= 12;
        signedTotal <<= 13;
        signedTotal >>= 14;
    }
}
"#;
        let mutations = collect(source);

        for value in ["unsignedAmount", "mask", "shift"] {
            assert!(mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Negate,
            )));
        }
        assert!(!mutations.contains(&assignment_mutation(
            source,
            "signedAmount",
            AssignmentReplacement::Negate,
        )));
        for value in ["11", "12"] {
            assert!(!mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Negate,
            )));
        }
        for value in ["13", "14"] {
            assert!(mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Negate,
            )));
        }
        for value in ["unsignedAmount", "signedAmount", "mask", "shift"] {
            assert!(!mutations.contains(&assignment_mutation(
                source,
                value,
                AssignmentReplacement::Zero,
            )));
        }
    }

    #[test]
    fn preserves_valid_signed_and_fixed_bytes_assignments() {
        let source = r#"
contract Test {
    function check(int256 signed, bytes32 word) external pure {
        int256 signedCopy = signed;
        bytes32 wordCopy = word;
    }
}
"#;
        let mutations = collect(source);

        for replacement in [AssignmentReplacement::Zero, AssignmentReplacement::Negate] {
            assert!(!mutations.contains(&assignment_mutation(source, "signed", replacement)));
        }
        assert!(!mutations.contains(&assignment_mutation(
            source,
            "word",
            AssignmentReplacement::Zero,
        )));
        assert!(mutations.contains(&assignment_mutation(
            source,
            "word",
            AssignmentReplacement::Negate,
        )));
    }

    #[test]
    fn preserves_udvt_assignments() {
        let source = r#"
type Amount is uint256;

contract Test {
    function check(Amount amount) external pure {
        Amount copy = amount;
    }
}
"#;
        let mutations = collect(source);

        for replacement in [AssignmentReplacement::Zero, AssignmentReplacement::Negate] {
            assert!(!mutations.contains(&assignment_mutation(source, "amount", replacement)));
        }
    }

    #[test]
    fn preserves_all_mutations_when_analysis_has_errors() {
        let source = r#"
contract Test {
    function check(address account) external pure {
        address copy = account;
        unresolved = unresolved;
    }
}
"#;

        assert!(collect(source).is_empty());
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
    fn excludes_lvalue_mutations_for_user_defined_push() {
        let source = r#"
contract Test {
    function push() external pure returns (int256) {
        return 1;
    }

    function check() external view returns (int256) {
        return -this.push();
    }
}
"#;
        let mutations = collect(source);

        for op in [UnOpKind::PreInc, UnOpKind::PreDec, UnOpKind::PostInc, UnOpKind::PostDec] {
            assert!(mutations.contains(&unary_mutation(source, "-this.push()", op)));
        }
    }

    #[test]
    fn preserves_lvalue_mutations_for_storage_push() {
        let source = r#"
contract Test {
    int256[] values;

    function check() external returns (int256) {
        return -values.push();
    }
}
"#;
        let mutations = collect(source);

        for op in [UnOpKind::PreInc, UnOpKind::PreDec, UnOpKind::PostInc, UnOpKind::PostDec] {
            assert!(!mutations.contains(&unary_mutation(source, "-values.push()", op)));
        }
    }

    #[test]
    fn excludes_invalid_fixed_bytes_unary_mutations() {
        let source = r#"
contract Test {
    function check(bytes32 word) external pure returns (bytes32) {
        return ~word;
    }

    function checkNumber(uint256 number) external pure returns (uint256) {
        return ~number;
    }
}
"#;
        let mutations = collect(source);

        for op in [
            UnOpKind::PreInc,
            UnOpKind::PreDec,
            UnOpKind::PostInc,
            UnOpKind::PostDec,
            UnOpKind::Neg,
        ] {
            assert!(mutations.contains(&unary_mutation(source, "~word", op)));
        }
        assert!(!mutations.contains(&unary_mutation(source, "~word", UnOpKind::BitNot)));

        for op in [UnOpKind::PreInc, UnOpKind::PreDec, UnOpKind::PostInc, UnOpKind::PostDec] {
            assert!(!mutations.contains(&unary_mutation(source, "~number", op)));
        }
        assert!(mutations.contains(&unary_mutation(source, "~number", UnOpKind::Neg)));
    }

    #[test]
    fn excludes_invalid_fixed_bytes_storage_push_mutations() {
        let source = r#"
contract Test {
    bytes32[] values;

    function check() external returns (bytes32) {
        return ~values.push();
    }
}
"#;
        let mutations = collect(source);

        for op in [
            UnOpKind::PreInc,
            UnOpKind::PreDec,
            UnOpKind::PostInc,
            UnOpKind::PostDec,
            UnOpKind::Neg,
        ] {
            assert!(mutations.contains(&unary_mutation(source, "~values.push()", op)));
        }
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
