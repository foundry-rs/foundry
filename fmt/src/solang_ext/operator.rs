use solang_parser::pt::*;

use super::{LineOfCode, OptionalLineOfCode};

/// The precedence of an Operator.
/// The lower the level, the sooner it will be evaluated.
/// E.g. P01 should be evaluated before P02
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Precedence {
    P00,
    P01,
    P02,
    P03,
    P04,
    P05,
    P06,
    P07,
    P08,
    P09,
    P10,
    P11,
    P12,
    P13,
    P14,
    P15,
}

impl Precedence {
    /// Is the precedence level evaluated from left to right
    fn is_left_to_right(self) -> bool {
        !matches!(self, Self::P03)
    }

    /// Is `self` as the left hand side evaluated before the right hand side
    pub fn is_evaluated_first(self, rhs: Precedence) -> bool {
        if self == rhs {
            self.is_left_to_right()
        } else {
            self < rhs
        }
    }
}

/// A simplified list of expressions grouped by operation precedence.
/// All expressions with groups will be evaluated first and then from left to right
pub enum FlatExpression<Expr> {
    Expression(Expr),
    SpacedOp(&'static str),
    UnspacedOp(&'static str),
    Group(Vec<FlatExpression<Expr>>),
}

impl<Expr> OptionalLineOfCode for FlatExpression<Expr>
where
    Expr: LineOfCode,
{
    fn loc(&self) -> Option<Loc> {
        match self {
            Self::Expression(expr) => Some(expr.loc()),
            Self::SpacedOp(_) | Self::UnspacedOp(_) => None,
            Self::Group(group) => {
                let mut locs = group.iter().filter_map(|item| item.loc());
                let mut first = locs.next()?;
                let last = locs.last().unwrap_or(first);
                first.use_end_from(&last);
                Some(first)
            }
        }
    }
}

/// Describes the precedence and the operator token of an Expression if it has one
pub trait Operator: Sized {
    fn precedence(&self) -> Precedence;
    fn operator(&self) -> Option<&'static str>;
    fn has_space_around(&self) -> bool;
    fn into_components(self) -> (Option<Self>, Option<Self>);
    fn flatten(self) -> Vec<FlatExpression<Self>> {
        let op = if let Some(op) = self.operator() {
            op
        } else {
            return vec![FlatExpression::Expression(self)]
        };
        let mut out = Vec::new();
        let has_space_around = self.has_space_around();
        let precedence = self.precedence();
        let (left, right) = self.into_components();

        if let Some(left) = left {
            if left.precedence().is_evaluated_first(precedence) {
                out.append(&mut left.flatten());
            } else {
                out.push(FlatExpression::Group(left.flatten()))
            }
            out.push(if has_space_around {
                FlatExpression::SpacedOp(op)
            } else {
                FlatExpression::UnspacedOp(op)
            });
            if let Some(right) = right {
                if precedence.is_evaluated_first(right.precedence()) {
                    out.push(FlatExpression::Group(right.flatten()))
                } else {
                    out.append(&mut right.flatten());
                }
            }
        } else if let Some(right) = right {
            out.push(if has_space_around {
                FlatExpression::SpacedOp(op)
            } else {
                FlatExpression::UnspacedOp(op)
            });
            if precedence.is_evaluated_first(right.precedence()) {
                out.push(FlatExpression::Group(right.flatten()))
            } else {
                out.append(&mut right.flatten());
            }
        }

        out
    }
}

impl Operator for &mut Expression {
    fn precedence(&self) -> Precedence {
        use Expression::*;
        use Precedence::*;
        match self {
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => P00,
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            PreIncrement(..) |
            PostIncrement(..) |
            New(..) |
            MemberAccess(..) => P01,
            Not(..) | Complement(..) | Delete(..) | UnaryPlus(..) | UnaryMinus(..) |
            PreDecrement(..) | PostDecrement(..) => P02,
            Power(..) => P03,
            Multiply(..) | Divide(..) | Modulo(..) => P04,
            Add(..) | Subtract(..) => P05,
            ShiftLeft(..) | ShiftRight(..) => P06,
            BitwiseAnd(..) => P07,
            BitwiseXor(..) => P08,
            BitwiseOr(..) => P09,
            Less(..) | More(..) | LessEqual(..) | MoreEqual(..) => P10,
            Equal(..) | NotEqual(..) => P11,
            And(..) => P12,
            Or(..) => P13,
            Ternary(..) => P14,
            Assign(..) | AssignOr(..) | AssignAnd(..) | AssignXor(..) | AssignShiftLeft(..) |
            AssignShiftRight(..) | AssignAdd(..) | AssignSubtract(..) | AssignMultiply(..) |
            AssignDivide(..) | AssignModulo(..) => P15,
        }
    }
    fn operator(&self) -> Option<&'static str> {
        use Expression::*;
        Some(match self {
            PreIncrement(..) | PostIncrement(..) => "++",
            PreDecrement(..) | PostDecrement(..) => "--",
            New(..) => "new",
            Not(..) => "!",
            Complement(..) => "~",
            Delete(..) => "delete",
            UnaryPlus(..) | Add(..) => "+",
            UnaryMinus(..) | Subtract(..) => "-",
            Power(..) => "**",
            Multiply(..) => "*",
            Divide(..) => "/",
            Modulo(..) => "%",
            ShiftLeft(..) => "<<",
            ShiftRight(..) => ">>",
            BitwiseAnd(..) => "&",
            BitwiseXor(..) => "^",
            BitwiseOr(..) => "|",
            Less(..) => "<",
            More(..) => ">",
            LessEqual(..) => "<=",
            MoreEqual(..) => ">=",
            And(..) => "&&",
            Or(..) => "||",
            Equal(..) => "==",
            NotEqual(..) => "!=",
            Assign(..) => "=",
            AssignOr(..) => "|=",
            AssignAnd(..) => "&=",
            AssignXor(..) => "^=",
            AssignShiftLeft(..) => "<<=",
            AssignShiftRight(..) => ">>=",
            AssignAdd(..) => "+=",
            AssignSubtract(..) => "-=",
            AssignMultiply(..) => "*=",
            AssignDivide(..) => "/=",
            AssignModulo(..) => "%=",
            MemberAccess(..) |
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            Ternary(..) |
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => return None,
        })
    }
    fn has_space_around(&self) -> bool {
        use Expression::*;
        !matches!(
            self,
            PostIncrement(..) |
                PreIncrement(..) |
                PostDecrement(..) |
                PreDecrement(..) |
                Not(..) |
                Complement(..) |
                UnaryPlus(..) |
                UnaryMinus(..)
        )
    }
    fn into_components(self) -> (Option<Self>, Option<Self>) {
        use Expression::*;
        match self {
            PostDecrement(_, expr) | PostIncrement(_, expr) => (Some(expr.as_mut()), None),
            Not(_, expr) |
            Complement(_, expr) |
            New(_, expr) |
            Delete(_, expr) |
            UnaryPlus(_, expr) |
            UnaryMinus(_, expr) |
            PreDecrement(_, expr) |
            PreIncrement(_, expr) => (None, Some(expr.as_mut())),
            Power(_, left, right) |
            Multiply(_, left, right) |
            Divide(_, left, right) |
            Modulo(_, left, right) |
            Add(_, left, right) |
            Subtract(_, left, right) |
            ShiftLeft(_, left, right) |
            ShiftRight(_, left, right) |
            BitwiseAnd(_, left, right) |
            BitwiseXor(_, left, right) |
            BitwiseOr(_, left, right) |
            Less(_, left, right) |
            More(_, left, right) |
            LessEqual(_, left, right) |
            MoreEqual(_, left, right) |
            Equal(_, left, right) |
            NotEqual(_, left, right) |
            And(_, left, right) |
            Or(_, left, right) |
            Assign(_, left, right) |
            AssignOr(_, left, right) |
            AssignAnd(_, left, right) |
            AssignXor(_, left, right) |
            AssignShiftLeft(_, left, right) |
            AssignShiftRight(_, left, right) |
            AssignAdd(_, left, right) |
            AssignSubtract(_, left, right) |
            AssignMultiply(_, left, right) |
            AssignDivide(_, left, right) |
            AssignModulo(_, left, right) => (Some(left.as_mut()), Some(right.as_mut())),
            MemberAccess(..) |
            Ternary(..) |
            ArraySubscript(..) |
            ArraySlice(..) |
            FunctionCall(..) |
            FunctionCallBlock(..) |
            NamedFunctionCall(..) |
            BoolLiteral(..) |
            NumberLiteral(..) |
            RationalNumberLiteral(..) |
            HexNumberLiteral(..) |
            StringLiteral(..) |
            Type(..) |
            HexLiteral(..) |
            AddressLiteral(..) |
            Variable(..) |
            List(..) |
            ArrayLiteral(..) |
            Unit(..) |
            This(..) => (None, None),
        }
    }
}
