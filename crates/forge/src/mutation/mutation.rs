// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar:
use solar_parse::{ast::{BinOpKind, Expr, ExprKind, IndexKind, LitKind, Span, TypeKind, UnOpKind, VariableDefinition}, interface::BytePos};
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

    /// For a binary op y in BinOpKind ("+", "-", ">=", etc)
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
    // This mutation is not used anymore, as we mutate the condition as an expression,
    // which will creates true/false mutant as well as more complex conditions (eg if(foo++ > --bar) )
    // IfStatementMutation,

    /// For a require(x) condition:
    /// replace x with true; replace x with false
    // same as IfStatementMutation, the expression inside the require is mutated as an expression
    // to handle increment etc
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

    /// For an unary operator x in UnOpKind (eg "++", "--", "~", "!"):
    /// replace x with all other operator in op
    /// Pre or post- are different UnOp
    UnaryOperatorMutation(UnOpKind),
}

enum MutationResult {
    Dead,
    Alive,
    Invalid,
}

/// A given mutation
#[derive(Debug)]
pub struct Mutant {
    span: Span,
    mutation: MutationType,
}

impl Mutant {
    pub fn new(span: Span, mutation: MutationType) -> Mutant {
        Mutant { span, mutation }
    }
}

pub trait Mutate {
    /// Return all the mutation which can be conducted against a given ExprKind
    fn get_all_mutations(&self) -> Option<Vec<Mutant>>;
}

impl<'ast> Mutate for Expr<'ast> {
    fn get_all_mutations(&self) -> Option<Vec<Mutant>> {
        let mut mutants = Vec::new();

        let _ = match &self.kind {
            // Array skipped for now (swap could be mutating it, cf above for rational)
            ExprKind::Assign(_, bin_op, rhs) => {
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
            ExprKind::Call(expr, args) => {
                if let ExprKind::Member(expr, ident) = &expr.kind {
                    if ident.to_string() == "delegatecall" {                    
                        mutants.push(create_delegatecall_mutation(ident.span));
                    }
                }
            }
            // CallOptions
            ExprKind::Delete(_) => mutants.push(create_delete_mutation(self.span)),
            // Indent
            // Index -> mutable? 0 it? idx should be a regular expression?
            // Lit -> global/constant are using Lit as initializer

            // Member
            // New
            // Payable -> compilation error
            // Ternary -> swap them?
            // Tuple -> swap if same type?
            // TypeCall -> compilation error
            // Type -> compilation error, most likely
            ExprKind::Unary(op, expr) => {
                mutants.push(create_unary_mutation(op.span, op.kind));
            }

            _ => {}
        };

        (!mutants.is_empty()).then_some(mutants)
    }
}

// @todo refactor:

fn create_assignement_mutation(span: Span, var_type: LitKind) -> Mutant {
    Mutant { span, mutation: MutationType::AssignmentMutation(var_type) }
}

fn create_binary_op_mutation(span: Span, op: BinOpKind) -> Mutant {
    Mutant { span, mutation: MutationType::BinaryOpMutation(op) }
}

fn create_delete_mutation(span: Span) -> Mutant {
    Mutant { span, mutation: MutationType::DeleteExpressionMutation}
}

fn create_unary_mutation(span: Span, op: UnOpKind) -> Mutant {
    Mutant { span, mutation: MutationType::UnaryOperatorMutation(op)}
}

fn create_delegatecall_mutation(span: Span) -> Mutant {
    Mutant { span, mutation: MutationType::ElimDelegateMutation }
}