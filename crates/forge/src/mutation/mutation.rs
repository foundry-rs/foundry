// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to
// select mutants) Use Solar:
use rand::{distributions::Alphanumeric, prelude::*, Rng};
use solar_parse::{
    ast::{
        BinOpKind, Expr, ExprKind, Ident, IndexKind, LitKind, Span, TypeKind, UnOp, UnOpKind,
        VariableDefinition,
    },
    interface::BytePos,
};
use std::path::PathBuf;

use super::visitor::AssignVarTypes;

/// Wraps an unary operator mutated, to easily store pre/post-fix op swaps
#[derive(Debug)]
struct UnaryOpMutated {
    /// String containing the whole new expression (operator and its target)
    /// eg `a++`
    new_expression: String,

    /// Span covering the whole non-mutated expression to cover (we might need to shrink or
    /// enlarge) eg from `a` to after the second `+` in `a++`
    span: Span,

    /// The underlying operator used by this mutant
    resulting_op_kind: UnOpKind,
}

impl UnaryOpMutated {
    fn new(new_expression: String, span: Span, resulting_op_kind: UnOpKind) -> Self {
        UnaryOpMutated { new_expression, span, resulting_op_kind }
    }
}

impl ToString for UnaryOpMutated {
    fn to_string(&self) -> String {
        self.new_expression.clone()
    }
}

#[derive(Debug)]
pub enum MutationType {
    // @todo Solar doesn't differentiate numeric type in LitKind (only on declaration?) -> for
    // now, planket and let solc filter out the invalid mutants -> we might/should add a
    // hashtable of the var to their underlying type (signed or not) so we avoid *a lot* of
    // invalid mutants
    /// For an initializer x, of type
    /// - bool: replace x with !x
    /// - uint: replace x with 0
    /// - int: replace x with 0; replace x with -x (temp: this is mutated for uint as well)
    /// For a binary op y: apply BinaryOpMutation(y)
    AssignmentMutation(AssignVarTypes),

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
    // which will creates true/false mutant as well as more complex conditions (eg if(foo++ > --bar)
    // ) IfStatementMutation,

    /// For a require(x) condition:
    /// replace x with true; replace x with false
    // Same as for IfStatementMutation, the expression inside the require is mutated as an
    // expression to handle increment etc
    RequireMutation,

    // @todo review if needed -> this might creates *a lot* of combinations for super-polyadic fn
    // tho       only swapping same type (to avoid obvious compilation failure), but should
    // take into account       implicit casting too...
    /// For 2 args of the same type x,y in a function args:
    /// swap(x, y)
    SwapArgumentsFunctionMutation,

    // @todo same remark as above, might end up in a space too big to explore + filtering out
    // based on type
    /// For an expr taking 2 expression x, y (x+y, x-y, x = x + ...):
    /// swap(x, y)
    SwapArgumentsOperatorMutation,

    /// For an unary operator x in UnOpKind (eg "++", "--", "~", "!"):
    /// replace x with all other operator in op
    /// Pre or post- are different UnOp
    UnaryOperatorMutation(UnaryOpMutated),
}

impl MutationType {
    fn get_name(&self) -> String {
        match self {
            MutationType::AssignmentMutation(var_type) => match var_type {
                AssignVarTypes::Literal(kind) => {
                    format!("{}_{}", "AssignmentMutation".to_string(), kind.description())
                }
                AssignVarTypes::Identifier(ident) => {
                    format!("{}_{}", "AssignmentMutation".to_string(), ident)
                }
            },
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
            MutationType::UnaryOperatorMutation(mutated) => {
                // avoid operator in tmp dir name
                format!("{}_{:?}", "UnaryOperatorMutation".to_string(), mutated.resulting_op_kind)
            }
        }
    }
}

impl ToString for MutationType {
    fn to_string(&self) -> String {
        match self {
            MutationType::AssignmentMutation(kind) => match kind {
                AssignVarTypes::Literal(kind) => match kind {
                    LitKind::Number(val) => val.to_string(),
                    _ => todo!(),
                },
                AssignVarTypes::Identifier(ident) => ident.as_str().to_owned(),
            },
            MutationType::BinaryOpMutation(kind) => kind.to_str().to_owned(),
            MutationType::UnaryOperatorMutation(mutated) => mutated.to_string(),
            _ => "".to_string(),
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
    pub fn create_assignement_mutation(span: Span, var_type: AssignVarTypes) -> Vec<Mutant> {
        match var_type {
            AssignVarTypes::Literal(lit) => match lit {
                LitKind::Bool(val) => vec![Mutant {
                    span,
                    mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                        LitKind::Bool(!val),
                    )),
                    path: PathBuf::default(),
                }],
                LitKind::Number(val) => {
                    vec![
                        Mutant {
                            span,
                            mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                                LitKind::Number(num_bigint::BigInt::ZERO),
                            )),
                            path: PathBuf::default(),
                        },
                        Mutant {
                            span,
                            mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                                LitKind::Number(-val),
                            )),
                            path: PathBuf::default(),
                        },
                    ]
                }
                _ => {
                    vec![]
                }
            },
            AssignVarTypes::Identifier(ident) => {
                let inner = ident.to_string();

                vec![
                    Mutant {
                        span,
                        mutation: MutationType::AssignmentMutation(AssignVarTypes::Literal(
                            LitKind::Number(num_bigint::BigInt::ZERO),
                        )),
                        path: PathBuf::default(),
                    },
                    Mutant {
                        span,
                        mutation: MutationType::AssignmentMutation(AssignVarTypes::Identifier(
                            format!("-{}", inner),
                        )),
                        path: PathBuf::default(),
                    },
                ]
            }
        }
    }

    pub fn create_binary_op_mutation(span: Span, op: BinOpKind) -> Vec<Mutant> {
        let operations_bools = vec![
            // Bool
            BinOpKind::Lt,
            BinOpKind::Le,
            BinOpKind::Gt,
            BinOpKind::Ge,
            BinOpKind::Eq,
            BinOpKind::Ne,
            BinOpKind::Or,
            BinOpKind::And,
        ];

        let operations_num_bitwise = vec![
            // Arithm
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

        let operations =
            if operations_bools.contains(&op) { operations_bools } else { operations_num_bitwise };

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

    /// The `target_expr` is the expresion the unary operator is modifying (ie `a` in `a++`)
    /// We only mutate where target_expr.kind is either a literal (123), an ident (foo) or a member
    /// (foo.x) original_span is the whole span (expr and op)
    pub fn create_unary_mutation(
        original_span: Span,
        op: UnOpKind,
        target_expr: &Expr<'_>,
    ) -> Vec<Mutant> {
        let operations = vec![
            UnOpKind::PreInc, // number
            UnOpKind::PreDec, // n
            UnOpKind::Not,    // b
            UnOpKind::Neg,    // n
            UnOpKind::BitNot, // n
        ];

        let post_fixed_operations = vec![UnOpKind::PostInc, UnOpKind::PostDec];

        let target_kind = &target_expr.kind;

        let target_content = match target_kind {
            ExprKind::Lit(lit, _) => match &lit.kind {
                LitKind::Bool(val) => val.to_string(),
                LitKind::Number(val) => val.to_string(),
                _ => "".to_string(),
            },
            ExprKind::Ident(inner) => inner.to_string(),
            ExprKind::Member(expr, ident) => {
                todo!()
            }
            _ => "".to_string(),
        };

        let mut mutations: Vec<Mutant>;

        mutations = operations
            .into_iter()
            .filter(|&kind| kind != op)
            .map(|kind| {
                let new_expression = format!("{}{}", kind.to_str(), target_content);

                let mutated = UnaryOpMutated::new(new_expression, original_span, kind);

                Mutant {
                    span: original_span,
                    mutation: MutationType::UnaryOperatorMutation(mutated),
                    path: PathBuf::default(),
                }
            })
            .collect();

        mutations.extend(post_fixed_operations.into_iter().filter(|&kind| kind != op).map(
            |kind| {
                let new_expression = format!("{}{}", target_content, kind.to_str());

                let mutated = UnaryOpMutated::new(new_expression, original_span, kind);

                Mutant {
                    span: original_span,
                    mutation: MutationType::UnaryOperatorMutation(mutated),
                    path: PathBuf::default(),
                }
            },
        ));

        return mutations;
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
