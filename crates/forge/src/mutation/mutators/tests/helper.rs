use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{assignement_mutator::AssignmentMutator, MutationContext, Mutator},
    visitor::AssignVarTypes,
    Session,
};
use num_bigint::BigInt;
use solar_parse::{
    ast::{Arena, Expr, ExprKind, Ident, Lit, LitKind, Span, Symbol},
    interface::BytePos,
};
use std::{collections::HashMap, hash::Hash, path::PathBuf};

// Create a span for test use
pub fn create_span(start: u32, end: u32) -> Span {
    Span::new(BytePos(start), BytePos(end))
}

// Create identifier with given name
pub fn create_ident(name: &str) -> Ident {
    Ident::from_str(name)
}

// Create number literal
pub fn create_number_lit(value: u32, span: Span) -> Lit {
    Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Number(value.into()) }
}

// Create boolean literal
pub fn create_bool_lit(value: bool, span: Span) -> Lit {
    Lit { span, symbol: Symbol::DUMMY, kind: LitKind::Bool(value) }
}

pub fn all_but_one<T: Eq + Hash>(theoretic: &[T], observed: &[T]) -> bool {
    if theoretic.len() != observed.len() + 1 {
        return false;
    }

    let mut counts = HashMap::new();

    for item in theoretic {
        *counts.entry(item).or_insert(0) += 1;
    }

    for item in observed {
        if let Some(count) = counts.get_mut(item) {
            *count -= 1;
            if *count == 0 {
                counts.remove(item);
            }
        } else {
            return false; // observed has something not in theoretic
        }
    }

    // Only one item should remain in the map
    counts.len() == 1 && counts.values().all(|&v| v == 1)
}
