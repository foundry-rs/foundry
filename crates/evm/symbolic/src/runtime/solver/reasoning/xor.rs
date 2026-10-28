//! A checked specialist for affine systems of EVM word XOR equalities.
//!
//! Gaussian elimination proposes either a model or a set of source equations whose XOR is
//! contradictory. Neither answer is trusted: models are evaluated against the entire original
//! query, and the certificate checker cancels terms in the original source equations without
//! consulting the elimination rows. Unsupported queries and exhausted work budgets fall through.

use super::*;

// A word-sized bitset tracks both columns and source equations. Bound expression unfolding too:
// a small hash-consed DAG can represent an exponentially large unfolded XOR tree.
const MAX_XOR_ROWS: usize = 256;
const MAX_XOR_ATOMS: usize = 256;
const MAX_XOR_VISITS: usize = 8192;

pub(in super::super) enum CheckedXorResult {
    Unsat,
    Model(SymbolicModel),
}

#[derive(Clone, Copy)]
struct Row {
    columns: U256,
    value: U256,
    sources: U256,
}

struct Relation<'a> {
    left: &'a SymExpr,
    right: &'a SymExpr,
    negated: bool,
}

/// Returns only independently checked answers, using the original, unnormalized constraints.
pub(in super::super) fn checked_xor_result(
    constraints: &[SymBoolExpr],
) -> Option<CheckedXorResult> {
    let relations = asserted_relations(constraints)?;
    // Most EVM queries are not XOR workloads. Do not run elimination for ordinary equalities.
    if !relations
        .iter()
        .any(|relation| affine_terms(relation).0.into_iter().flatten().any(is_affine_op))
    {
        return None;
    }

    let mut atoms = HashMap::<&SymExpr, usize>::default();
    let mut basis = [None::<Row>; MAX_XOR_ATOMS];
    let mut visits = MAX_XOR_VISITS;
    let mut rows = Vec::with_capacity(relations.len());
    for (index, relation) in relations.iter().enumerate() {
        let (terms, constant) = affine_terms(relation);
        let mut row = Row { columns: U256::ZERO, value: constant, sources: U256::ONE << index };
        let mut pending = terms.into_iter().flatten().collect::<Vec<_>>();
        while let Some(expr) = pending.pop() {
            visits = visits.checked_sub(1)?;
            match expr.kind() {
                SymExprKind::Const(value) => row.value ^= *value,
                SymExprKind::BinOp(SymBinOp::Xor, left, right) => {
                    pending.extend([left, right]);
                }
                SymExprKind::Not(value) => {
                    row.value ^= U256::MAX;
                    pending.push(value);
                }
                _ => {
                    let next = atoms.len();
                    let column = *atoms.entry(expr).or_insert(next);
                    if column >= MAX_XOR_ATOMS {
                        return None;
                    }
                    row.columns ^= U256::ONE << column;
                }
            }
        }
        rows.push(row);
    }
    for (relation, mut row) in relations.iter().zip(rows.iter().copied()) {
        if relation.negated {
            continue;
        }
        while !row.columns.is_zero() {
            let pivot = row.columns.trailing_zeros();
            if let Some(previous) = basis[pivot] {
                row.columns ^= previous.columns;
                row.value ^= previous.value;
                row.sources ^= previous.sources;
            } else {
                basis[pivot] = Some(row);
                break;
            }
        }
        if row.columns.is_zero() && !row.value.is_zero() {
            return check_certificate(&relations, row.sources, None)
                .then_some(CheckedXorResult::Unsat);
        }
    }
    // A disequality is impossible when its equality follows from the asserted equalities.
    // Disequalities never become basis rows: combining them as equations would be unsound.
    for (index, (relation, mut row)) in relations.iter().zip(rows).enumerate() {
        if !relation.negated {
            continue;
        }
        row.sources = U256::ZERO;
        while !row.columns.is_zero() {
            let pivot = row.columns.trailing_zeros();
            let Some(previous) = basis[pivot] else { break };
            row.columns ^= previous.columns;
            row.value ^= previous.value;
            row.sources ^= previous.sources;
        }
        if row.columns.is_zero() && row.value.is_zero() {
            return check_certificate(&relations, row.sources, Some(index))
                .then_some(CheckedXorResult::Unsat);
        }
    }

    // Free columns are zero. Back-substitution assigns pivot columns; opaque expressions are
    // never independently assignable, so only variable columns enter the candidate model.
    if constraints.iter().any(SymBoolExpr::contains_gasleft) {
        return None;
    }
    let mut values = [U256::ZERO; MAX_XOR_ATOMS];
    for (pivot, row) in basis.iter().enumerate().rev() {
        if let Some(row) = row {
            let mut value = row.value;
            for (column, assigned) in values.iter().enumerate().skip(pivot + 1) {
                if row.columns.bit(column) {
                    value ^= *assigned;
                }
            }
            values[pivot] = value;
        }
    }
    let model = atoms
        .into_iter()
        .filter_map(|(expr, column)| match expr.kind() {
            SymExprKind::Var(symbol) => Some((*symbol, values[column])),
            _ => None,
        })
        .collect();
    eval_model_constraints(constraints, &model).then_some(CheckedXorResult::Model(model))
}

/// Collects asserted equalities and disequalities. Negated conjunctions cannot supply relations.
fn asserted_relations(constraints: &[SymBoolExpr]) -> Option<Vec<Relation<'_>>> {
    if constraints.len() > MAX_XOR_VISITS {
        return None;
    }
    let mut relations = Vec::new();
    let mut pending = constraints.iter().collect::<Vec<_>>();
    let mut visits = MAX_XOR_VISITS;
    while let Some(constraint) = pending.pop() {
        visits = visits.checked_sub(1)?;
        if let SymBoolExprKind::And(values) = constraint.kind() {
            if pending.len() + values.len() > visits {
                return None;
            }
            pending.extend(values.iter());
            continue;
        }
        let (value, negated) = match constraint.kind() {
            SymBoolExprKind::Not(value) => (value, true),
            _ => (constraint, false),
        };
        if let SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) = value.kind() {
            if relations.len() == MAX_XOR_ROWS {
                return None;
            }
            relations.push(Relation { left, right, negated });
        }
    }
    Some(relations)
}

fn is_affine_op(expr: &SymExpr) -> bool {
    matches!(expr.kind(), SymExprKind::BinOp(SymBinOp::Xor, _, _) | SymExprKind::Not(_))
}

/// Recovers equalities compiled as `ISZERO(ADD(word, constant))` or `ISZERO(SUB(a, b))`.
/// Only the equality boundary admits these rewrites: addition inside an XOR remains opaque
/// because carries are not linear over GF(2). Every constant operation wraps at EVM word width.
fn affine_terms<'a>(relation: &Relation<'a>) -> ([Option<&'a SymExpr>; 2], U256) {
    let Some(mut target) = relation.right.as_const() else {
        return ([Some(relation.left), Some(relation.right)], U256::ZERO);
    };
    let mut word = relation.left;
    for _ in 0..32 {
        match word.kind() {
            SymExprKind::BinOp(SymBinOp::Add, left, right)
                if let Some(value) = right.as_const() =>
            {
                target = target.wrapping_sub(value);
                word = left;
            }
            SymExprKind::BinOp(SymBinOp::Sub, left, right) => {
                if let Some(value) = right.as_const() {
                    target = target.wrapping_add(value);
                    word = left;
                } else if let Some(value) = left.as_const() {
                    target = value.wrapping_sub(target);
                    word = right;
                } else if target.is_zero() {
                    return ([Some(left), Some(right)], U256::ZERO);
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    ([Some(word), None], target)
}

/// Checks a contradiction certificate by XORing selected source equalities directly. All opaque
/// terms must cancel, leaving `0 = nonzero`, or `0 != 0` for a disequality certificate. This is a
/// necessary contradiction even when atoms are related (e.g. `x` and `x & mask`); no independence
/// assumption is used to establish UNSAT.
fn check_certificate(
    relations: &[Relation<'_>],
    sources: U256,
    disequality: Option<usize>,
) -> bool {
    if relations.len() < MAX_XOR_ROWS && !(sources >> relations.len()).is_zero() {
        return false;
    }
    if disequality
        .is_some_and(|index| !relations.get(index).is_some_and(|relation| relation.negated))
    {
        return false;
    }
    let mut terms = HashSet::<&SymExpr>::default();
    let mut constant = U256::ZERO;
    let mut visits = MAX_XOR_VISITS;
    for (index, relation) in relations.iter().enumerate() {
        if sources.bit(index) && relation.negated {
            return false;
        }
        if !sources.bit(index) && disequality != Some(index) {
            continue;
        }
        let (words, offset) = affine_terms(relation);
        constant ^= offset;
        let mut pending = words.into_iter().flatten().collect::<Vec<_>>();
        while let Some(expr) = pending.pop() {
            let Some(remaining) = visits.checked_sub(1) else { return false };
            visits = remaining;
            match expr.kind() {
                SymExprKind::Const(value) => constant ^= *value,
                SymExprKind::BinOp(SymBinOp::Xor, left, right) => {
                    pending.extend([left, right]);
                }
                SymExprKind::Not(value) => {
                    constant ^= U256::MAX;
                    pending.push(value);
                }
                _ => {
                    if !terms.remove(expr) {
                        terms.insert(expr);
                    }
                }
            }
        }
    }
    terms.is_empty() && constant.is_zero() == disequality.is_some()
}
