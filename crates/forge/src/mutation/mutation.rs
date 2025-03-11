// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to select mutants)
// Use Solar:
use rand::prelude::*;
use rand::{distributions::Alphanumeric, Rng};
use solar_parse::{
    ast::{
        BinOpKind, Expr, ExprKind, IndexKind, LitKind, Span, TypeKind, UnOpKind, VariableDefinition,
    },
    interface::BytePos,
};
use std::path::PathBuf;

/// Kinds of mutations (taken from Certora's Gambit)
// #[derive(Hash, Eq, PartialEq, Clone, Copy)]
#[derive(Debug)]
pub enum MutationType {
    // @todo Solar doesn't differentiate numeric type in LitKind (only on declaration?) -> for now, planket and let solc filter out the invalid mutants
    // -> we might/should add a hashtable of the var to their underlying type (signed or not) so we avoid *a lot* of invalid mutants
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
    // Same as for IfStatementMutation, the expression inside the require is mutated as an expression
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
    UnaryOperatorMutation(UnOpKind, Span),
}

impl MutationType {
    fn get_name(&self) -> String {
        match self {
            MutationType::AssignmentMutation(kind) => {
                format!("{}_{}", "AssignmentMutation".to_string(), kind.description())
            }
            MutationType::BinaryOpMutation(kind) => {
                format!("{}_{:?}", "BinaryOpMutation".to_string(), kind)
            }
            MutationType::DeleteExpressionMutation => "DeleteExpressionMutation".to_string(),
            MutationType::ElimDelegateMutation => "ElimDelegateMutation".to_string(),
            MutationType::FunctionCallMutation => "FunctionCallMutation".to_string(),
            MutationType::RequireMutation => "RequireMutation".to_string(),
            MutationType::SwapArgumentsFunctionMutation => {
                "SwapArgumentsFunctionMutation".to_string()
            }
            MutationType::SwapArgumentsOperatorMutation => {
                "SwapArgumentsOperatorMutation".to_string()
            }
            MutationType::UnaryOperatorMutation(kind, _) => {
                format!("{}_{:?}", "UnaryOperatorMutation".to_string(), kind)
            }
        }
    }
}

#[derive(Debug)]
pub enum MutationResult {
    Dead,
    Alive,
    Invalid,
}

/// A given mutation
#[derive(Debug)]
pub struct Mutant {
    /// The path to the project root where this mutant (tries to) live
    pub path: PathBuf,
    pub span: Span,
    pub mutation: MutationType,
}

impl Mutant {
    pub fn create_assignement_mutation(span: Span, var_type: LitKind) -> Vec<Mutant> {
        match var_type {
            LitKind::Bool(val) => vec![Mutant {
                span,
                mutation: MutationType::AssignmentMutation(LitKind::Bool(!val)),
                path: PathBuf::default(),
            }],
            LitKind::Number(val) => {
                vec![
                    Mutant {
                        span,
                        mutation: MutationType::AssignmentMutation(LitKind::Number(
                            num_bigint::BigInt::ZERO,
                        )),
                        path: PathBuf::default(),
                    },
                    Mutant {
                        span,
                        mutation: MutationType::AssignmentMutation(LitKind::Number(-val)),
                        path: PathBuf::default(),
                    },
                ]
            }
            _ => {
                vec![]
            }
        }
    }

    pub fn create_binary_op_mutation(span: Span, op: BinOpKind) -> Vec<Mutant> {
        let operations = vec![
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
            BinOpKind::Or,
            BinOpKind::And,
            BinOpKind::Shr,
            BinOpKind::Shl,
            BinOpKind::Sar,
            BinOpKind::BitAnd,
            BinOpKind::BitOr,
            BinOpKind::BitXor,
            BinOpKind::Add,
            BinOpKind::Sub,
            BinOpKind::Pow,
            BinOpKind::Mul,
            BinOpKind::Div,
            BinOpKind::Rem,
        ];

        operations
            .into_iter()
            .filter(|&kind| kind != op)
            .map(|kind| Mutant {
                span,
                mutation: MutationType::BinaryOpMutation(kind),
                path: PathBuf::default(),
            })
            .collect()
    }

    pub fn create_delete_mutation(span: Span) -> Mutant {
        Mutant { span, mutation: MutationType::DeleteExpressionMutation, path: PathBuf::default() }
    }

    /// @dev the emitter will have to put pre and post-op before or after the target span
    /// eg ++a -> preInc, so should have --a as mutant, but a++ as well (target_span is the span for `a`)
    pub fn create_unary_mutation(span: Span, op: UnOpKind, target_span: Span) -> Vec<Mutant> {
        let operations = vec![
            UnOpKind::PreInc,
            UnOpKind::PreDec,
            UnOpKind::PostInc,
            UnOpKind::PostDec,
            UnOpKind::Not,
            UnOpKind::Neg,
            UnOpKind::BitNot,
        ];

        operations
            .into_iter()
            .filter(|&kind| kind != op)
            .map(|kind| Mutant {
                span,
                mutation: MutationType::UnaryOperatorMutation(kind, target_span),
                path: PathBuf::default(),
            })
            .collect()
    }

    pub fn create_delegatecall_mutation(span: Span) -> Mutant {
        Mutant { span, mutation: MutationType::ElimDelegateMutation, path: PathBuf::default() }
    }

    /// Get a temp folder name based on the span and the mutation to conduct
    pub fn get_unique_id(&self) -> String {
        format!(
            "{}_{}_{}",
            self.span.hi().to_u32(),
            self.span.lo().to_u32(),
            self.mutation.get_name()
        )
    }
}
