//! Routes each solver query to the solver most likely to answer it quickly.
//!
//! No single solver wins on every query: local search finds models of large `ite`-heavy queries,
//! integer blasting proves range-bounded multiplication and division, and bit blasting handles
//! the rest. A few cheap features of the query pick a bucket; each bucket has its own solver order,
//! refined online by how each solver did on earlier queries of that bucket.

use super::*;

/// A query whose expression graph has at least this many `ite` nodes counts as `ite`-heavy.
const ITE_HEAVY_NODES: usize = 256;

/// Coarse class of a solver query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum QueryBucket {
    /// Multiplies, divides or remainders two symbolic words.
    Nonlinear,
    /// Large `ite`-heavy query, as symbolic offsets and lengths produce.
    IteHeavy,
    /// Anything else.
    Plain,
}

impl QueryBucket {
    /// Every bucket, in the order the scheduler stores them.
    pub(crate) const ALL: [Self; 3] = [Self::Nonlinear, Self::IteHeavy, Self::Plain];

    /// Returns this bucket's index in [`Self::ALL`].
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Nonlinear => 0,
            Self::IteHeavy => 1,
            Self::Plain => 2,
        }
    }

    /// Returns the rank of a known solver in this bucket's default order, lower first. Unknown
    /// and unranked solvers come after the ranked ones, in configured order.
    pub(crate) fn prior_rank(self, name: Option<&str>) -> usize {
        let order: &[&str] = match self {
            Self::Nonlinear => {
                &["cvc5-int", "bitwuzla", "bitwuzla-abs", "yices", "z3", "z3-inc", "cvc5"]
            }
            Self::IteHeavy => &[
                "bitwuzla",
                "bitwuzla-prop",
                "bitwuzla-abs",
                "yices",
                "cvc5",
                "z3-inc",
                "z3",
                "cvc5-int",
            ],
            Self::Plain => {
                &["bitwuzla", "bitwuzla-abs", "yices", "z3", "z3-inc", "cvc5", "cvc5-int"]
            }
        };
        name.and_then(|name| order.iter().position(|known| *known == name)).unwrap_or(order.len())
    }
}

/// Returns the bucket of a normalized query.
pub(crate) fn query_bucket(constraints: &[SymBoolExpr]) -> QueryBucket {
    let mut nonlinear = false;
    let mut ites = 0usize;
    for constraint in constraints {
        let _ = visit_unique::<()>(None, Some(constraint), |expr| {
            match expr.kind() {
                SymExprKind::BinOp(
                    SymBinOp::Mul
                    | SymBinOp::UDiv
                    | SymBinOp::URem
                    | SymBinOp::SDiv
                    | SymBinOp::SRem,
                    left,
                    right,
                ) if left.as_const().is_none() && right.as_const().is_none() => nonlinear = true,
                SymExprKind::TernOp(SymTernOp::MulMod, ..) => nonlinear = true,
                SymExprKind::Ite(..) => ites += 1,
                _ => {}
            }
            ControlFlow::Continue(())
        });
    }
    if nonlinear {
        QueryBucket::Nonlinear
    } else if ites >= ITE_HEAVY_NODES {
        QueryBucket::IteHeavy
    } else {
        QueryBucket::Plain
    }
}
