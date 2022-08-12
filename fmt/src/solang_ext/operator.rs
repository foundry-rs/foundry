use solang_parser::pt::*;

/// Describes the precedence and the operator token of an Expression if it has one
pub trait Operator: Sized {
    fn unsplittable(&self) -> bool;
    fn operator(&self) -> Option<&'static str>;
    fn has_space_around(&self) -> bool;
    fn into_components(self) -> (Option<Self>, Option<Self>);
}

impl Operator for &mut Expression {
    fn unsplittable(&self) -> bool {
        use Expression::*;
        matches!(
            self,
            BoolLiteral(..) |
                NumberLiteral(..) |
                RationalNumberLiteral(..) |
                HexNumberLiteral(..) |
                StringLiteral(..) |
                HexLiteral(..) |
                AddressLiteral(..) |
                Variable(..) |
                Unit(..) |
                This(..)
        )
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
            This(..) |
            Parenthesis(..) => return None,
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
            Parenthesis(_, expr) |
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
