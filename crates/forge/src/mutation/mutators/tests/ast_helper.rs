use crate::mutation::{
    mutant::{Mutant, MutationType},
    mutators::{assignement_mutator::AssignmentMutator, MutationContext, Mutator},
    visitor::AssignVarTypes,
};
use solar_parse::{
    ast::{Arena, Expr, ExprKind, Ident, Lit, LitKind, Span, Symbol},
    interface::BytePos,
};

use num_bigint::BigInt;
use std::path::PathBuf;

use crate::mutation::Session;

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
