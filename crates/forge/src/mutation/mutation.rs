// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar: 
use solar_parse::
    ast::{
        Span, Expr, VariableDefinition
    };
use std::hash::Hash;

/// Kinds of mutations (taken from Certora's Gambit)
#[derive(Hash, Eq, PartialEq, Clone, Copy)]
pub enum MutationType {
    /// For an initializer x, of type
    /// - bool: replace x with !x
    /// - uint: replace x with 0
    /// - int: replace x with 0; replace x with -x
    /// For a binary op y: apply BinaryOpMutation(y)
    AssignmentMutation,

    /// For a binary op y in op=["+", "-", "*", "/", "%", "**"]:
    /// replace y with each non-y in op
    BinaryOpMutation,

    /// For a delete expr x `delete foo`, replace x with `assert(true)`
    DeleteExpressionMutation,

    /// replace "delegatecall" with "call"
    ElimDelegateMutation,

    /// Gambit doesn't implement nor define it?
    FunctionCallMutation,

    /// For a if(x) condition x:
    /// replace x with true; replace x with false
    IfStatementMutation,

    /// For a require(x) condition:
    /// replace x with true; replace x with false
    RequireMutation,

    // @todo review if needed -> this might creates *a lot* of combinations for super-polyadic fn tho
    //       only swapping same type (to avoid obvious compilation failure), but should take into account
    //       implicit casting too...
    /// For 2 args of the same type x,y in a function args:
    /// swap(x, y)
    SwapArgumentsFunctionMutation,

    // @todo same remark as above, might end up in a space too big to explore + filtering out based on type
    /// For an expr taking 2 expression x, y (x+y, x-y, x = x + ...):
    /// swap(x, y)
    SwapArgumentsOperatorMutation,
    
    // @todo pre and post-op should be different (and mutation would switch pre/post too)
    //       AST itself doesn't store this -> should be based on span (ie UnOp.span > Expr.span?)
    /// For an unary operator x in op=["++", "--", "~", "!"]:
    /// replace x with all other operator in op
    UnaryOperatorMutation,
}

enum MutationResult {
    Dead,
    Alive,
    Invalid
}

/// A given mutant and its faith
pub struct Mutant {
    mutation: MutationType,
    span: Span,
    outcome: MutationResult
}

pub trait Mutate {
    /// Return all the mutation which can be conducted against a given ExprKind
    fn get_all_mutations(&self) -> Option<Vec<Mutant>>;
}

impl<'ast> Mutate for Expr<'ast> {
    fn get_all_mutations(&self) -> Option<Vec<Mutant>> {
        dbg!(&self.kind);
        None
    }

}

impl<'ast> Mutate for VariableDefinition<'ast> {
    fn get_all_mutations(&self) -> Option<Vec<Mutant>> {
        dbg!(self.name);
        None
    }
}