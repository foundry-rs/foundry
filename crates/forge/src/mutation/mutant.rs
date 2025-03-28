// Generate mutants then run tests (reuse the whole unit test flow for now, including compilation to
// select mutants) Use Solar:
use super::visitor::AssignVarTypes;
use solar_parse::ast::{BinOpKind, LitKind, Span, UnOpKind};
use std::{fmt::Display, path::PathBuf};

/// Wraps an unary operator mutated, to easily store pre/post-fix op swaps
#[derive(Debug)]
pub struct UnaryOpMutated {
    /// String containing the whole new expression (operator and its target)
    /// eg `a++`
    new_expression: String,

    /// The underlying operator used by this mutant
    pub resulting_op_kind: UnOpKind,
}

impl UnaryOpMutated {
    pub fn new(new_expression: String, resulting_op_kind: UnOpKind) -> Self {
        Self { new_expression, resulting_op_kind }
    }
}

impl Display for UnaryOpMutated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.new_expression)
    }
}

// @todo add a mutation from universalmutator: line swap (swap two lines of code, as it
// could theoretically uncover untested reentrancies
#[derive(Debug)]
pub enum MutationType {
    // @todo Solar doesn't differentiate numeric type in LitKind (only on declaration?) -> for
    // now, planket and let solc filter out the invalid mutants -> we might/should add a
    // hashtable of the var to their underlying type (signed or not) so we avoid *a lot* of
    // invalid mutants
    /// For an initializer x, of type
    /// bool: replace x with !x
    /// uint: replace x with 0
    /// int: replace x with 0; replace x with -x (temp: this is mutated for uint as well)
    ///
    /// For a binary op y: apply BinaryOp(y)
    Assignment(AssignVarTypes),

    /// For a binary op y in BinOpKind ("+", "-", ">=", etc)
    /// replace y with each non-y in op
    BinaryOp(BinOpKind),

    /// For a delete expr x `delete foo`, replace x with `assert(true)`
    DeleteExpression,

    /// replace "delegatecall" with "call"
    ElimDelegate,

    /// Gambit doesn't implement nor define it?
    FunctionCall,

    // /// For a if(x) condition x:
    // /// replace x with true; replace x with false
    // This mutation is not used anymore, as we mutate the condition as an expression,
    // which will creates true/false mutant as well as more complex conditions (eg if(foo++ >
    // --bar) ) IfStatementMutation,
    /// For a require(x) condition:
    /// replace x with true; replace x with false
    // Same as for IfStatementMutation, the expression inside the require is mutated as an
    // expression to handle increment etc
    Require,

    // @todo review if needed -> this might creates *a lot* of combinations for super-polyadic fn
    // tho       only swapping same type (to avoid obvious compilation failure), but should
    // take into account       implicit casting too...
    /// For 2 args of the same type x,y in a function args:
    /// swap(x, y)
    SwapArgumentsFunction,

    // @todo same remark as above, might end up in a space too big to explore + filtering out
    // based on type
    /// For an expr taking 2 expression x, y (x+y, x-y, x = x + ...):
    /// swap(x, y)
    SwapArgumentsOperator,

    /// For an unary operator x in UnOpKind (eg "++", "--", "~", "!"):
    /// replace x with all other operator in op
    /// Pre or post- are different UnOp
    UnaryOperator(UnaryOpMutated),
}

impl MutationType {
    fn get_name(&self) -> String {
        match self {
            Self::Assignment(var_type) => match var_type {
                AssignVarTypes::Literal(kind) => {
                    format!("{}_{}", "Assignment", kind.description())
                }
                AssignVarTypes::Identifier(ident) => {
                    format!("{}_{}", "Assignment", ident)
                }
            },
            Self::BinaryOp(kind) => {
                format!("{}_{:?}", "BinaryOp", kind)
            }
            Self::DeleteExpression => "DeleteExpression".to_string(),
            Self::ElimDelegate => "ElimDelegate".to_string(),
            Self::FunctionCall => "FunctionCall".to_string(),
            Self::Require => "Require".to_string(),
            Self::SwapArgumentsFunction => "SwapArgumentsFunction".to_string(),
            Self::SwapArgumentsOperator => "SwapArgumentsOperator".to_string(),
            Self::UnaryOperator(mutated) => {
                // avoid operator in tmp dir name
                format!("{}_{:?}", "UnaryOperator", mutated.resulting_op_kind)
            }
        }
    }
}

impl Display for MutationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Assignment(kind) => match kind {
                AssignVarTypes::Literal(kind) => match kind {
                    LitKind::Number(val) => write!(f, "{val}"),
                    _ => todo!(),
                },
                AssignVarTypes::Identifier(ident) => write!(f, "{ident}"),
            },
            Self::BinaryOp(kind) => write!(f, "{}", kind.to_str()),
            Self::DeleteExpression => write!(f, "assert(true)"),
            Self::ElimDelegate => write!(f, "call"),
            Self::UnaryOperator(mutated) => write!(f, "{mutated}"),

            Self::FunctionCall |
            Self::Require |
            Self::SwapArgumentsFunction |
            Self::SwapArgumentsOperator => write!(f, ""),
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
