// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar:
use solar_parse::ast::{Expr, ExprKind, LitKind, Span, TypeKind, VariableDefinition, BinOpKind, IndexKind};
use std::hash::Hash;

/// Kinds of mutations (taken from Certora's Gambit)
// #[derive(Hash, Eq, PartialEq, Clone, Copy)]
#[derive(Debug)]
pub enum MutationType {
    // @todo Solar doesn't differentiate numeric type -> for now, planket and let solc filter out the invalid mutants
    /// For an initializer x, of type
    /// - bool: replace x with !x
    /// - uint: replace x with 0
    /// - int: replace x with 0; replace x with -x (temp: this is mutated for uint as well)
    /// For a binary op y: apply BinaryOpMutation(y)
    AssignmentMutation(LitKind),

    /// For a binary op y in op=["+", "-", "*", "/", "%", "**"]:
    /// replace y with each non-y in op
    BinaryOpMutation(BinOpKind),

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
    Invalid,
}

/// A given mutation
#[derive(Debug)]
pub struct Mutant {
    mutation: MutationType,
    span: Span,
}

pub trait Mutate {
    /// Return all the mutation which can be conducted against a given ExprKind
    fn get_all_mutations(&self) -> Option<Vec<Mutant>>;
}

impl<'ast> Mutate for Expr<'ast> {
    fn get_all_mutations(&self) -> Option<Vec<Mutant>> {
        let mut mutants = Vec::new();

        dbg!(&self.kind);
        let _ = match &self.kind {
            // Array skipped for now (swap could be mutating it, cf above for rational)
            ExprKind::Assign(_, bin_op, rhs) => {
                // mutants.push(create_assignement_mutation(rhs.span, rhs.kind));

                if let ExprKind::Lit(kind, _) = &rhs.kind {
                    mutants.push(create_assignement_mutation(rhs.span, kind.kind.clone()));
                }
                
                // @todo I think we should match other ones here too, for x = y++; for instance
                // match &rhs.kind {
                //     ExprKind::Lit(kind, _) => match &kind.kind {
                //         _ => { mutants.push(create_assignement_mutation(rhs.span, kind.kind.clone())) }
                //     },
                //     _ => {}
                // }
                
                if let Some(op) = &bin_op {
                    mutants.push(create_binary_op_mutation(op.span, op.kind));
                }

            },
            ExprKind::Binary(_, op, _) => {
                // @todo is a >> b++ a thing (ie parse lhs and rhs too?)
                mutants.push(create_binary_op_mutation(op.span, op.kind));
            },
            // Call
            // CallOptions
            ExprKind::Delete(_) => mutants.push(create_delete_mutation(self.span)),
            // Indet
            // Index -> mutable? 0 it? idx should be a regular expression?

            _ => {}
        };

        (!mutants.is_empty()).then_some(mutants)
    }
}

fn create_assignement_mutation(span: Span, var_type: LitKind) -> Mutant {
    Mutant { mutation: MutationType::AssignmentMutation(var_type), span }
}

fn create_binary_op_mutation(span: Span, op: BinOpKind) -> Mutant {
    Mutant { mutation: MutationType::BinaryOpMutation(op), span }
}

fn create_delete_mutation(span: Span) -> Mutant {
    Mutant { mutation: MutationType::DeleteExpressionMutation, span}
}